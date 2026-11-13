use aes_gcm::{AeadInOut, Aes128Gcm, Aes256Gcm, KeyInit, Nonce, Tag};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256, Sha384};

const HANDSHAKE_FINISHED: u8 = 20;
const CONTENT_HANDSHAKE: u8 = 0x16;
const RECORD_APPDATA: u8 = 0x17;

#[derive(Clone, Copy)]
enum Hash {
    Sha256,
    Sha384,
}

impl Hash {
    fn len(self) -> usize {
        match self {
            Self::Sha256 => 32,
            Self::Sha384 => 48,
        }
    }

    fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Sha256::digest(data).to_vec(),
            Self::Sha384 => Sha384::digest(data).to_vec(),
        }
    }

    fn extract(self, salt: &[u8], ikm: &[u8]) -> Vec<u8> {
        match self {
            Self::Sha256 => Hkdf::<Sha256>::extract(Some(salt), ikm).0.to_vec(),
            Self::Sha384 => Hkdf::<Sha384>::extract(Some(salt), ikm).0.to_vec(),
        }
    }

    fn expand(self, prk: &[u8], info: &[u8], output: &mut [u8]) -> Option<()> {
        match self {
            Self::Sha256 => Hkdf::<Sha256>::from_prk(prk)
                .ok()?
                .expand(info, output)
                .ok(),
            Self::Sha384 => Hkdf::<Sha384>::from_prk(prk)
                .ok()?
                .expand(info, output)
                .ok(),
        }
    }

    fn hmac(self, key: &[u8], data: &[u8]) -> Option<Vec<u8>> {
        Some(match self {
            Self::Sha256 => <Hmac<Sha256>>::new_from_slice(key)
                .ok()?
                .chain_update(data)
                .finalize()
                .into_bytes()
                .to_vec(),
            Self::Sha384 => <Hmac<Sha384>>::new_from_slice(key)
                .ok()?
                .chain_update(data)
                .finalize()
                .into_bytes()
                .to_vec(),
        })
    }

    fn expand_label(
        self,
        secret: &[u8],
        label: &[u8],
        context: &[u8],
        length: usize,
    ) -> Option<Vec<u8>> {
        let mut info = Vec::with_capacity(4 + 6 + label.len() + context.len());
        info.extend_from_slice(&(length as u16).to_be_bytes());
        info.push((6 + label.len()) as u8);
        info.extend_from_slice(b"tls13 ");
        info.extend_from_slice(label);
        info.push(context.len() as u8);
        info.extend_from_slice(context);
        let mut output = vec![0u8; length];
        self.expand(secret, &info, &mut output)?;
        Some(output)
    }
}

enum Cipher {
    Aes128(Box<Aes128Gcm>),
    Aes256(Box<Aes256Gcm>),
}

struct Protection {
    cipher: Cipher,
    iv: [u8; 12],
    sequence: u64,
}

impl Protection {
    fn new(hash: Hash, key_len: usize, secret: &[u8]) -> Option<Self> {
        let key = hash.expand_label(secret, b"key", b"", key_len)?;
        let iv: [u8; 12] = hash.expand_label(secret, b"iv", b"", 12)?.try_into().ok()?;
        let cipher = match key_len {
            16 => Cipher::Aes128(Box::new(Aes128Gcm::new_from_slice(&key).ok()?)),
            _ => Cipher::Aes256(Box::new(Aes256Gcm::new_from_slice(&key).ok()?)),
        };
        Some(Self {
            cipher,
            iv,
            sequence: 0,
        })
    }

    fn nonce(&mut self) -> Nonce<aes_gcm::aead::consts::U12> {
        let mut nonce = self.iv;
        for (byte, sequence) in nonce[4..].iter_mut().zip(self.sequence.to_be_bytes()) {
            *byte ^= sequence;
        }
        self.sequence += 1;
        Nonce::from(nonce)
    }

    fn open(&mut self, header: &[u8], body: &[u8]) -> Option<Vec<u8>> {
        let split = body.len().checked_sub(16)?;
        let mut plain = body[..split].to_vec();
        let tag = Tag::from(<[u8; 16]>::try_from(&body[split..]).ok()?);
        let nonce = self.nonce();
        match &self.cipher {
            Cipher::Aes128(aead) => aead
                .decrypt_inout_detached(&nonce, header, (&mut plain[..]).into(), &tag)
                .ok()?,
            Cipher::Aes256(aead) => aead
                .decrypt_inout_detached(&nonce, header, (&mut plain[..]).into(), &tag)
                .ok()?,
        }
        Some(plain)
    }

    fn seal(&mut self, content: &[u8], kind: u8) -> Option<Vec<u8>> {
        let mut record = vec![RECORD_APPDATA, 0x03, 0x03];
        record.extend_from_slice(&((content.len() + 1 + 16) as u16).to_be_bytes());
        let mut plain = content.to_vec();
        plain.push(kind);
        let nonce = self.nonce();
        let tag = match &self.cipher {
            Cipher::Aes128(aead) => aead
                .encrypt_inout_detached(&nonce, &record, (&mut plain[..]).into())
                .ok()?,
            Cipher::Aes256(aead) => aead
                .encrypt_inout_detached(&nonce, &record, (&mut plain[..]).into())
                .ok()?,
        };
        record.extend_from_slice(&plain);
        record.extend_from_slice(&tag);
        Some(record)
    }
}

pub(super) struct ClientHandshake {
    hash: Hash,
    transcript: Vec<u8>,
    client_secret: Vec<u8>,
    server: Protection,
    client: Protection,
    pending: Vec<u8>,
}

pub(super) enum Step {
    More,
    Finished(Vec<u8>),
}

impl ClientHandshake {
    pub(super) fn new(
        cipher_suite: u16,
        client_hello: &[u8],
        server_hello: &[u8],
        shared: &[u8],
    ) -> Option<Self> {
        let (hash, key_len) = match cipher_suite {
            0x1301 => (Hash::Sha256, 16),
            0x1302 => (Hash::Sha384, 32),
            _ => return None,
        };
        let mut transcript = client_hello.to_vec();
        transcript.extend_from_slice(server_hello);
        let zeros = vec![0u8; hash.len()];
        let early = hash.extract(&[0u8], &zeros);
        let derived = hash.expand_label(&early, b"derived", &hash.digest(b""), hash.len())?;
        let handshake = hash.extract(&derived, shared);
        let context = hash.digest(&transcript);
        let client_secret = hash.expand_label(&handshake, b"c hs traffic", &context, hash.len())?;
        let server_secret = hash.expand_label(&handshake, b"s hs traffic", &context, hash.len())?;
        Some(Self {
            hash,
            transcript,
            server: Protection::new(hash, key_len, &server_secret)?,
            client: Protection::new(hash, key_len, &client_secret)?,
            client_secret,
            pending: Vec::new(),
        })
    }

    pub(super) fn server_record(&mut self, header: &[u8], body: &[u8]) -> Option<Step> {
        let mut plain = self.server.open(header, body)?;
        while plain.last() == Some(&0) {
            plain.pop();
        }
        if plain.pop()? != CONTENT_HANDSHAKE {
            return None;
        }
        self.pending.extend_from_slice(&plain);
        while self.pending.len() >= 4 {
            let length = usize::from(self.pending[1]) << 16
                | usize::from(self.pending[2]) << 8
                | usize::from(self.pending[3]);
            if self.pending.len() < 4 + length {
                break;
            }
            let message: Vec<u8> = self.pending.drain(..4 + length).collect();
            self.transcript.extend_from_slice(&message);
            if message[0] == HANDSHAKE_FINISHED {
                return self.client_finished().map(Step::Finished);
            }
        }
        Some(Step::More)
    }

    fn client_finished(&mut self) -> Option<Vec<u8>> {
        let key = self
            .hash
            .expand_label(&self.client_secret, b"finished", b"", self.hash.len())?;
        let verify = self.hash.hmac(&key, &self.hash.digest(&self.transcript))?;
        let mut message = vec![HANDSHAKE_FINISHED, 0, 0, verify.len() as u8];
        message.extend_from_slice(&verify);
        let mut flight = vec![0x14, 0x03, 0x03, 0x00, 0x01, 0x01];
        flight.extend_from_slice(&self.client.seal(&message, CONTENT_HANDSHAKE)?);
        Some(flight)
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    #[test]
    fn the_key_schedule_matches_rfc_8448() {
        let hash = Hash::Sha256;
        let early = hash.extract(&[0u8], &[0u8; 32]);
        assert_eq!(
            hex::encode(&early),
            "33ad0a1c607ec03b09e6cd9893680ce210adf300aa1f2660e1b22e10f170f92a"
        );
        let derived = hash
            .expand_label(&early, b"derived", &hash.digest(b""), 32)
            .unwrap();
        assert_eq!(
            hex::encode(&derived),
            "6f2615a108c702c5678f54fc9dbab69716c076189c48250cebeac3576c3611ba"
        );
        let shared =
            hex::decode("8bd4054fb55b9d63fdfbacf9f04b9f0d35e6d63f537563efd46272900f89492d")
                .unwrap();
        assert_eq!(
            hex::encode(hash.extract(&derived, &shared)),
            "1dc826e93606aa6fdc0aadc12f741b01046aa6b99f691ed221a9f0ca043fbeac"
        );
    }

    pub(in crate::transport) struct Server {
        hash: Hash,
        transcript: Vec<u8>,
        server_secret: Vec<u8>,
        client_secret: Vec<u8>,
        server: Protection,
        client: Protection,
    }

    impl Server {
        pub(in crate::transport) fn new(
            cipher_suite: u16,
            client_hello: &[u8],
            server_hello: &[u8],
            shared: &[u8],
        ) -> Self {
            let client = ClientHandshake::new(cipher_suite, client_hello, server_hello, shared)
                .expect("supported suite");
            let hash = client.hash;
            let key_len = if cipher_suite == 0x1301 { 16 } else { 32 };
            let zeros = vec![0u8; hash.len()];
            let early = hash.extract(&[0u8], &zeros);
            let derived = hash
                .expand_label(&early, b"derived", &hash.digest(b""), hash.len())
                .unwrap();
            let handshake = hash.extract(&derived, shared);
            let context = hash.digest(&client.transcript);
            let server_secret = hash
                .expand_label(&handshake, b"s hs traffic", &context, hash.len())
                .unwrap();
            Self {
                hash,
                transcript: client.transcript.clone(),
                server: Protection::new(hash, key_len, &server_secret).unwrap(),
                client: Protection::new(hash, key_len, &client.client_secret).unwrap(),
                client_secret: client.client_secret,
                server_secret,
            }
        }

        pub(in crate::transport) fn flight(&mut self, certificate: usize) -> Vec<u8> {
            let mut flight = vec![0x14, 0x03, 0x03, 0x00, 0x01, 0x01];
            let extensions = vec![8u8, 0, 0, 2, 0, 0];
            let mut certificate_message = vec![11u8, 0, 0, 0];
            certificate_message.extend(std::iter::repeat_n(0x42, certificate));
            let length = certificate.to_be_bytes();
            certificate_message[1..4].copy_from_slice(&length[length.len() - 3..]);
            for message in [extensions, certificate_message] {
                self.transcript.extend_from_slice(&message);
                flight.extend_from_slice(&self.server.seal(&message, CONTENT_HANDSHAKE).unwrap());
            }
            let key = self
                .hash
                .expand_label(&self.server_secret, b"finished", b"", self.hash.len())
                .unwrap();
            let verify = self
                .hash
                .hmac(&key, &self.hash.digest(&self.transcript))
                .unwrap();
            let mut finished = vec![HANDSHAKE_FINISHED, 0, 0, verify.len() as u8];
            finished.extend_from_slice(&verify);
            self.transcript.extend_from_slice(&finished);
            flight.extend_from_slice(&self.server.seal(&finished, CONTENT_HANDSHAKE).unwrap());
            flight
        }

        pub(in crate::transport) fn check_client_finished(
            &mut self,
            header: &[u8],
            body: &[u8],
        ) -> bool {
            let Some(mut plain) = self.client.open(header, body) else {
                return false;
            };
            if plain.pop() != Some(CONTENT_HANDSHAKE) {
                return false;
            }
            let key = self
                .hash
                .expand_label(&self.client_secret, b"finished", b"", self.hash.len())
                .unwrap();
            let verify = self
                .hash
                .hmac(&key, &self.hash.digest(&self.transcript))
                .unwrap();
            plain[4..] == verify[..]
        }
    }
}
