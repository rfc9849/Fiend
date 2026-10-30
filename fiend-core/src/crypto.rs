use aes_gcm::{AeadInOut, Aes256Gcm, KeyInit, Nonce, Tag};
use hkdf::Hkdf;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

use std::fmt;

type HmacSha256 = Hmac<Sha256>;

const LABEL_SERVER_AUTH: &[u8] = b"sh-auth";
const LABEL_HANDSHAKE_DONE: &[u8] = b"handshake-done";
const LABEL_CLIENT_FINISHED: &[u8] = b"client-finished";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("session key derivation failed")]
    KeyDerivation,
    #[error("record encryption failed")]
    Encrypt,
    #[error("record decryption failed")]
    Decrypt,
}

fn hmac_sha256(secret: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    mac.finalize().into_bytes().into()
}

fn hmac_sha256_matches(secret: &[u8], parts: &[&[u8]], expected: &[u8]) -> bool {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    for part in parts {
        mac.update(part);
    }
    mac.verify_slice(expected).is_ok()
}

pub fn auth_token(secret: &[u8], tls_random: &[u8], hour_bucket: i64) -> [u8; 32] {
    hmac_sha256(secret, &[tls_random, &(hour_bucket as u64).to_be_bytes()])
}

pub fn auth_token_matches(
    secret: &[u8],
    tls_random: &[u8],
    hour_bucket: i64,
    supplied: &[u8],
) -> bool {
    hmac_sha256_matches(
        secret,
        &[tls_random, &(hour_bucket as u64).to_be_bytes()],
        supplied,
    )
}

#[derive(Clone)]
pub struct AuthKey(HmacSha256);

impl AuthKey {
    pub fn new(secret: &[u8]) -> Self {
        Self(HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length"))
    }

    pub fn matches(&self, tls_random: &[u8], hour_bucket: i64, supplied: &[u8]) -> bool {
        let mut mac = self.0.clone();
        mac.update(tls_random);
        mac.update(&(hour_bucket as u64).to_be_bytes());
        mac.verify_slice(supplied).is_ok()
    }
}

pub const SERVER_NONCE_SIZE: usize = 16;

pub fn server_auth_random(
    secret: &[u8],
    client_random: &[u8],
    nonce: &[u8; SERVER_NONCE_SIZE],
) -> [u8; 32] {
    let tag = hmac_sha256(secret, &[LABEL_SERVER_AUTH, client_random, nonce]);
    let mut random = [0u8; 32];
    random[..SERVER_NONCE_SIZE].copy_from_slice(nonce);
    random[SERVER_NONCE_SIZE..].copy_from_slice(&tag[..32 - SERVER_NONCE_SIZE]);
    random
}

pub fn server_auth_random_matches(secret: &[u8], client_random: &[u8], supplied: &[u8]) -> bool {
    if supplied.len() != 32 {
        return false;
    }
    let (nonce, tag) = supplied.split_at(SERVER_NONCE_SIZE);
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(LABEL_SERVER_AUTH);
    mac.update(client_random);
    mac.update(nonce);
    mac.verify_truncated_left(tag).is_ok()
}

pub fn handshake_terminator(
    secret: &[u8],
    client_random: &[u8],
    server_random: &[u8],
    version: u8,
) -> [u8; 32] {
    hmac_sha256(
        secret,
        &[
            LABEL_HANDSHAKE_DONE,
            &[version],
            client_random,
            server_random,
        ],
    )
}

pub fn handshake_terminator_matches(
    secret: &[u8],
    client_random: &[u8],
    server_random: &[u8],
    version: u8,
    supplied: &[u8],
) -> bool {
    hmac_sha256_matches(
        secret,
        &[
            LABEL_HANDSHAKE_DONE,
            &[version],
            client_random,
            server_random,
        ],
        supplied,
    )
}

pub fn client_finished(secret: &[u8], client_random: &[u8], server_random: &[u8]) -> [u8; 32] {
    hmac_sha256(
        secret,
        &[LABEL_CLIENT_FINISHED, client_random, server_random],
    )
}

pub fn client_finished_matches(
    secret: &[u8],
    client_random: &[u8],
    server_random: &[u8],
    supplied: &[u8],
) -> bool {
    hmac_sha256_matches(
        secret,
        &[LABEL_CLIENT_FINISHED, client_random, server_random],
        supplied,
    )
}

pub struct RecordCipher {
    aead: Aes256Gcm,
    iv: [u8; 12],
    counter: u64,
}

impl fmt::Debug for RecordCipher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordCipher")
            .field("counter", &self.counter)
            .finish_non_exhaustive()
    }
}

impl RecordCipher {
    pub fn new(key: &[u8; 32], iv: [u8; 12]) -> Self {
        let aead = Aes256Gcm::new_from_slice(key).expect("AES-256 key is 32 bytes");
        Self {
            aead,
            iv,
            counter: 0,
        }
    }

    fn nonce(&self) -> [u8; 12] {
        let mut nonce = self.iv;
        let prefix = u64::from_le_bytes(self.iv[..8].try_into().expect("eight bytes"));
        nonce[..8].copy_from_slice(&(prefix ^ self.counter).to_le_bytes());
        nonce
    }

    pub fn encrypt(&mut self, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
        let mut output = Vec::with_capacity(plaintext.len() + 16);
        self.seal_into(plaintext, &mut output)?;
        Ok(output)
    }

    pub fn decrypt(&mut self, ciphertext: &[u8]) -> Result<Vec<u8>, Error> {
        let mut output = ciphertext.to_vec();
        self.open_in_place(&mut output)?;
        Ok(output)
    }

    pub fn seal_into(&mut self, plaintext: &[u8], output: &mut Vec<u8>) -> Result<(), Error> {
        let start = output.len();
        output.extend_from_slice(plaintext);
        let nonce = self.nonce();
        let tag = self
            .aead
            .encrypt_inout_detached(&Nonce::from(nonce), b"", (&mut output[start..]).into())
            .map_err(|_| Error::Encrypt)?;
        output.extend_from_slice(&tag);
        self.counter += 1;
        Ok(())
    }

    pub fn open_in_place(&mut self, buffer: &mut Vec<u8>) -> Result<(), Error> {
        if buffer.len() < 16 {
            return Err(Error::Decrypt);
        }
        let plaintext_len = buffer.len() - 16;
        let mut tag_bytes = [0u8; 16];
        tag_bytes.copy_from_slice(&buffer[plaintext_len..]);
        let nonce = self.nonce();
        let tag = Tag::from(tag_bytes);
        self.aead
            .decrypt_inout_detached(
                &Nonce::from(nonce),
                b"",
                (&mut buffer[..plaintext_len]).into(),
                &tag,
            )
            .map_err(|_| Error::Decrypt)?;
        buffer.truncate(plaintext_len);
        self.counter += 1;
        Ok(())
    }
}

pub struct SessionKeyPair {
    pub c2s: RecordCipher,
    pub s2c: RecordCipher,
}

impl SessionKeyPair {
    pub fn for_client(self) -> (RecordCipher, RecordCipher) {
        (self.c2s, self.s2c)
    }

    pub fn for_server(self) -> (RecordCipher, RecordCipher) {
        (self.s2c, self.c2s)
    }
}

pub fn derive_session_keys(
    secret: &[u8],
    client_random: &[u8],
    server_random: &[u8],
    shared: &[u8],
) -> Result<SessionKeyPair, Error> {
    let mut ikm = Zeroizing::new(Vec::with_capacity(secret.len() + shared.len()));
    ikm.extend_from_slice(secret);
    ikm.extend_from_slice(shared);
    let mut salt = Vec::with_capacity(client_random.len() + server_random.len());
    salt.extend_from_slice(client_random);
    salt.extend_from_slice(server_random);

    let hkdf = Hkdf::<Sha256>::new(Some(&salt), &ikm);
    let expand = |label: &[u8], output: &mut [u8]| {
        hkdf.expand(label, output).map_err(|_| Error::KeyDerivation)
    };

    let mut c2s_key = Zeroizing::new([0u8; 32]);
    let mut s2c_key = Zeroizing::new([0u8; 32]);
    let mut c2s_iv = [0u8; 12];
    let mut s2c_iv = [0u8; 12];
    expand(b"c2s-key", &mut c2s_key[..])?;
    expand(b"s2c-key", &mut s2c_key[..])?;
    expand(b"c2s-iv", &mut c2s_iv)?;
    expand(b"s2c-iv", &mut s2c_iv)?;

    Ok(SessionKeyPair {
        c2s: RecordCipher::new(&c2s_key, c2s_iv),
        s2c: RecordCipher::new(&s2c_key, s2c_iv),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: [u8; 32] = [
        0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e,
        0x0f, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x1b, 0x1c, 0x1d,
        0x1e, 0x1f,
    ];

    #[test]
    fn auth_token_matches_reference() {
        let token = auth_token(&SECRET, &SECRET, 1);
        assert_eq!(
            hex::encode(token),
            "a8ed855e67786baacef4af2762cf437db63f3643a71426f4b2ebc0f4a27c2ed3"
        );
    }

    #[test]
    fn handshake_macs_match_reference() {
        let client = [0x11; 32];
        let server = [0x22; 32];
        let nonce = [0x44; SERVER_NONCE_SIZE];
        let random = server_auth_random(&SECRET, &client, &nonce);
        assert_eq!(random[..SERVER_NONCE_SIZE], nonce);
        assert_ne!(
            random,
            server_auth_random(&SECRET, &client, &[0x45; SERVER_NONCE_SIZE])
        );
        assert_eq!(
            hex::encode(handshake_terminator(&SECRET, &client, &server, 3)),
            "8d21e99ce5cf99bb5d0415594c1446f714e69e58d4509c9ec96505e9c57e0d7b"
        );
        assert_eq!(
            hex::encode(client_finished(&SECRET, &client, &server)),
            "80f81ef6d887bca4ff1223df6d914f0901df0105d22ff2cfeeb49d533da2aa7a"
        );

        assert!(server_auth_random_matches(&SECRET, &client, &random));
        let mut tampered = random;
        tampered[0] ^= 1;
        assert!(!server_auth_random_matches(&SECRET, &client, &tampered));
        assert!(!server_auth_random_matches(&SECRET, &[0x12; 32], &random));
        assert!(!server_auth_random_matches(&SECRET, &client, &[0u8; 32]));
        assert!(client_finished_matches(
            &SECRET,
            &client,
            &server,
            &client_finished(&SECRET, &client, &server)
        ));
    }

    #[test]
    fn session_keys_match_reference() {
        let client = [0x11; 32];
        let server = [0x22; 32];
        let shared = [0x33; 32];
        let mut pair =
            derive_session_keys(&SECRET, &client, &server, &shared).expect("derive keys");
        assert_eq!(
            hex::encode(pair.c2s.encrypt(b"carrier").expect("encrypt")),
            "c2b25e64718853b0fd5ff90e8cdb9be9de16b01de2ae91"
        );
        let encrypted = pair.s2c.encrypt(b"reply").expect("encrypt reply");
        let mut read = RecordCipher::new(
            &decode32("ecbdee363d8db66ae8654bab25441c6e825e5b686502ec592d7f4a6095e07c06"),
            decode12("b90c035772ff25e350d856bf"),
        );
        assert_eq!(read.decrypt(&encrypted).expect("decrypt"), b"reply");
    }

    #[test]
    fn record_cipher_round_trips_and_rejects_tamper() {
        let key = [7u8; 32];
        let iv = [9u8; 12];
        let mut write = RecordCipher::new(&key, iv);
        let mut read = RecordCipher::new(&key, iv);

        let first = write.encrypt(b"first record").expect("encrypt first");
        let second = write.encrypt(b"second record").expect("encrypt second");
        assert_ne!(first, second);
        assert_eq!(
            read.decrypt(&first).expect("decrypt first"),
            b"first record"
        );
        assert_eq!(
            read.decrypt(&second).expect("decrypt second"),
            b"second record"
        );

        let mut tampered = write.encrypt(b"tampered").expect("encrypt third");
        tampered[0] ^= 1;
        assert!(matches!(read.decrypt(&tampered), Err(Error::Decrypt)));
    }

    fn decode32(hex_value: &str) -> [u8; 32] {
        hex::decode(hex_value)
            .expect("valid hex")
            .try_into()
            .expect("32 bytes")
    }

    fn decode12(hex_value: &str) -> [u8; 12] {
        hex::decode(hex_value)
            .expect("valid hex")
            .try_into()
            .expect("12 bytes")
    }
}
