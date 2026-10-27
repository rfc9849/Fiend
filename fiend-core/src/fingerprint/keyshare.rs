use std::fmt;

use ml_kem::{DecapsulationKey768, KeyExport, Seed};
use p256::SecretKey;
use p256::elliptic_curve::sec1::ToSec1Point;
use x25519_dalek::{EphemeralSecret, PublicKey};

use super::error::{Error, Result};

pub const X25519: u16 = 0x001d;
pub const P256: u16 = 0x0017;
pub const X25519_MLKEM768: u16 = 0x11ec;

pub const X25519_SIZE: usize = 32;
pub const P256_SIZE: usize = 65;
pub const X25519_MLKEM768_SIZE: usize = 1216;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyShareSpec {
    pub group: u16,
    pub offset: usize,
    pub size: usize,
    pub public: Vec<u8>,
}

pub enum KeyShareSecret {
    X25519(EphemeralSecret),
    P256(SecretKey),
    X25519MlKem768 {
        mlkem: DecapsulationKey768,
        x25519: EphemeralSecret,
    },
}

pub struct GeneratedKeyShare {
    pub group: u16,
    pub secret: KeyShareSecret,
    pub public: Vec<u8>,
}

impl fmt::Debug for KeyShareSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::X25519(_) => f.write_str("X25519(..)"),
            Self::P256(_) => f.write_str("P256(..)"),
            Self::X25519MlKem768 { .. } => f.write_str("X25519MlKem768(..)"),
        }
    }
}

impl fmt::Debug for GeneratedKeyShare {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GeneratedKeyShare")
            .field("group", &self.group)
            .field("secret", &self.secret)
            .field("public", &hex::encode(&self.public))
            .finish()
    }
}

#[derive(Default)]
pub struct KeyShares {
    pub shares: Vec<GeneratedKeyShare>,
}

impl fmt::Debug for KeyShares {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(&self.shares).finish()
    }
}

pub fn generate(group: u16) -> Result<GeneratedKeyShare> {
    match group {
        X25519 => {
            let secret = EphemeralSecret::random_from_rng(&mut rand::rng());
            let public = PublicKey::from(&secret).as_bytes().to_vec();
            Ok(GeneratedKeyShare {
                group,
                secret: KeyShareSecret::X25519(secret),
                public,
            })
        }
        P256 => {
            let secret = loop {
                let mut bytes = [0u8; 32];
                rand::fill(&mut bytes);
                if let Ok(secret) = SecretKey::from_slice(&bytes) {
                    break secret;
                }
            };
            let public = secret.public_key().to_sec1_point(false).as_bytes().to_vec();
            Ok(GeneratedKeyShare {
                group,
                secret: KeyShareSecret::P256(secret),
                public,
            })
        }
        X25519_MLKEM768 => {
            let mut seed = Seed::default();
            rand::fill(&mut seed[..]);
            let mlkem = DecapsulationKey768::from_seed(seed);
            let x25519 = EphemeralSecret::random_from_rng(&mut rand::rng());
            let mut public = Vec::with_capacity(X25519_MLKEM768_SIZE);
            public.extend_from_slice(mlkem.encapsulation_key().to_bytes().as_slice());
            public.extend_from_slice(PublicKey::from(&x25519).as_bytes());
            Ok(GeneratedKeyShare {
                group,
                secret: KeyShareSecret::X25519MlKem768 { mlkem, x25519 },
                public,
            })
        }
        0x6399 => Err(Error::invalid(
            "X25519Kyber768Draft00 (0x6399) is unsupported; the Kyber draft is dead, regenerate the template",
        )),
        _ => Err(Error::UnsupportedKeyShareGroup(group)),
    }
}

pub fn expected_size(group: u16) -> Option<usize> {
    match group {
        X25519 => Some(X25519_SIZE),
        P256 => Some(P256_SIZE),
        X25519_MLKEM768 => Some(X25519_MLKEM768_SIZE),
        _ => None,
    }
}

pub fn validate_group(group: u16, size: usize) -> Result<()> {
    match group {
        X25519 if size == X25519_SIZE => Ok(()),
        P256 if size == P256_SIZE => Ok(()),
        X25519_MLKEM768 if size == X25519_MLKEM768_SIZE => Ok(()),
        0x6399 => Err(Error::invalid(
            "X25519Kyber768Draft00 (0x6399) is unsupported; the Kyber draft is dead, regenerate the template",
        )),
        X25519 | P256 | X25519_MLKEM768 => Err(Error::invalid(format!(
            "key_share group 0x{group:04x} must be {} bytes, got {size}",
            expected_size(group).expect("known group")
        ))),
        _ => Err(Error::UnsupportedKeyShareGroup(group)),
    }
}

pub fn is_grease(value: u16) -> bool {
    value & 0x0f0f == 0x0a0a && value >> 8 == value & 0xff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_shares_have_template_sizes() {
        let x25519 = generate(X25519).expect("generate X25519");
        assert_eq!(x25519.public.len(), X25519_SIZE);

        let p256 = generate(P256).expect("generate P256");
        assert_eq!(p256.public.len(), P256_SIZE);
        assert_eq!(p256.public[0], 0x04);

        let hybrid = generate(X25519_MLKEM768).expect("generate hybrid");
        assert_eq!(hybrid.public.len(), X25519_MLKEM768_SIZE);
    }

    #[test]
    fn known_groups_reject_wrong_sizes() {
        assert!(validate_group(X25519, 31).is_err());
        assert!(validate_group(P256, 64).is_err());
        assert!(validate_group(X25519_MLKEM768, 1215).is_err());
        assert!(validate_group(0x6399, 1216).is_err());
        assert!(validate_group(0x1234, 32).is_err());
    }

    #[test]
    fn grease_detection_matches_tls_values() {
        assert!(is_grease(0x0a0a));
        assert!(is_grease(0x1a1a));
        assert!(is_grease(0xfafa));
        assert!(!is_grease(0x0a0b));
        assert!(!is_grease(0x1301));
    }
}
