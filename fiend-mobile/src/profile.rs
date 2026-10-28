use std::io::{Read, Write};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use fiend_core::client::ClientConfig;
use fiend_core::fingerprint::{Template, parse_client_hello};
use flate2::{Compression, bufread::ZlibDecoder, write::ZlibEncoder};
use serde::{Deserialize, Serialize};

use crate::FiendError;

const PREFIX: &str = "fiend://v1/";
const MAX_LINK: usize = 65536;
const MAX_JSON: usize = 262144;
const MAX_TEMPLATE: usize = 131072;
const BUNDLED: &str = include_str!("../../fiend-core/testdata/template.json");

#[derive(Clone, uniffi::Record)]
pub struct ProfileConfig {
    pub name: String,
    pub server: String,
    pub secret: String,
    pub template_json: String,
}

#[derive(uniffi::Record)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub template_json: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ShareProfile {
    name: String,
    server: String,
    secret: String,
    template: serde_json::Value,
}

pub(crate) fn client_config(profile: &ProfileConfig) -> Result<ClientConfig, FiendError> {
    if profile.name.trim().is_empty()
        || profile.name.len() > 128
        || profile.name.chars().any(char::is_control)
    {
        return Err(invalid(
            "Profile name must contain 1 to 128 bytes and no control characters",
        ));
    }
    if profile.secret.len() != 64 {
        return Err(invalid("Secret must contain 64 hex characters"));
    }
    let secret = hex::decode(&profile.secret).map_err(|_| invalid("Secret is not valid hex"))?;
    if profile.template_json.len() > MAX_TEMPLATE {
        return Err(invalid("Template is too large"));
    }
    let template = Template::from_json(&profile.template_json)
        .map_err(|_| invalid("Template is not valid"))?;
    let captured = parse_client_hello(template.raw_bytes())
        .map_err(|_| invalid("Template capture is not valid"))?;
    if template.domain != captured.domain
        || template.tls_random != captured.tls_random
        || template.session_id != captured.session_id
        || template.key_share != captured.key_share
        || template.psk_identity != captured.psk_identity
        || template.ticket_age != captured.ticket_age
        || template.psk_binder != captured.psk_binder
    {
        return Err(invalid("Template fields do not match its capture"));
    }
    fiend_core::config::build_client(ClientConfig::new(&profile.server, secret, template))
        .map_err(|error| invalid(error.to_string()))
}

#[uniffi::export]
pub fn validate_profile(profile: ProfileConfig) -> Result<(), FiendError> {
    client_config(&profile).map(drop)
}

#[uniffi::export]
pub fn make_share_link(profile: ProfileConfig) -> Result<String, FiendError> {
    client_config(&profile)?;
    let share = ShareProfile {
        name: profile.name,
        server: profile.server.trim().to_owned(),
        secret: profile.secret.to_ascii_lowercase(),
        template: serde_json::from_str(&profile.template_json)
            .map_err(|_| invalid("Template is not valid"))?,
    };
    let raw = serde_json::to_vec(&share).map_err(|_| FiendError::InvalidLink)?;
    if raw.len() > MAX_JSON {
        return Err(FiendError::InvalidLink);
    }
    let mut compressor = ZlibEncoder::new(Vec::new(), Compression::best());
    compressor
        .write_all(&raw)
        .map_err(|_| FiendError::InvalidLink)?;
    let compressed = compressor.finish().map_err(|_| FiendError::InvalidLink)?;
    let link = format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(compressed));
    if link.len() > MAX_LINK {
        return Err(FiendError::InvalidLink);
    }
    Ok(link)
}

#[uniffi::export]
pub fn parse_share_link(uri: String) -> Result<ProfileConfig, FiendError> {
    if uri.len() > MAX_LINK {
        return Err(FiendError::InvalidLink);
    }
    let encoded = uri.strip_prefix(PREFIX).ok_or(FiendError::InvalidLink)?;
    let compressed = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|_| FiendError::InvalidLink)?;
    let mut decoder = ZlibDecoder::new(compressed.as_slice());
    let mut raw = Vec::new();
    decoder
        .by_ref()
        .take(MAX_JSON as u64 + 1)
        .read_to_end(&mut raw)
        .map_err(|_| FiendError::InvalidLink)?;
    if raw.len() > MAX_JSON || !decoder.get_ref().is_empty() {
        return Err(FiendError::InvalidLink);
    }
    let share: ShareProfile = serde_json::from_slice(&raw).map_err(|_| FiendError::InvalidLink)?;
    let profile = ProfileConfig {
        name: share.name,
        server: share.server,
        secret: share.secret,
        template_json: serde_json::to_string(&share.template)
            .map_err(|_| FiendError::InvalidLink)?,
    };
    client_config(&profile)?;
    Ok(profile)
}

#[uniffi::export]
pub fn bundled_templates() -> Vec<TemplateInfo> {
    vec![TemplateInfo {
        id: "captured-wikipedia-v1".into(),
        name: "Captured TLS template".into(),
        platform: "TLS".into(),
        template_json: BUNDLED.into(),
    }]
}

fn invalid(reason: impl Into<String>) -> FiendError {
    FiendError::InvalidProfile {
        reason: reason.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn profile() -> ProfileConfig {
        ProfileConfig {
            name: "Local test".into(),
            server: "127.0.0.1:443".into(),
            secret: "42".repeat(32),
            template_json: BUNDLED.into(),
        }
    }

    #[test]
    fn share_link_round_trip_keeps_capture_and_secret() {
        let profile = profile();
        let link = make_share_link(profile.clone()).unwrap();
        assert!(link.starts_with(PREFIX));
        assert!(!link.contains('='));
        let restored = parse_share_link(link.clone()).unwrap();
        assert_eq!(restored.name, profile.name);
        assert_eq!(restored.server, profile.server);
        assert_eq!(restored.secret, profile.secret);
        assert_eq!(
            client_config(&restored).unwrap().template.raw,
            client_config(&profile).unwrap().template.raw
        );
        assert_eq!(make_share_link(restored).unwrap(), link);
        eprintln!("bundled-template share link: {} bytes", link.len());
    }

    #[test]
    fn links_reject_bombs_trailing_data_and_unknown_versions() {
        for link in [
            "fiend://v2/abc",
            "https://example.com",
            "fiend://v1/!",
            "fiend://v1/AAAA",
        ] {
            assert!(parse_share_link(link.into()).is_err());
        }
        let link = make_share_link(profile()).unwrap();
        let mut data = URL_SAFE_NO_PAD
            .decode(link.strip_prefix(PREFIX).unwrap())
            .unwrap();
        data.push(0);
        assert!(parse_share_link(format!("{PREFIX}{}", URL_SAFE_NO_PAD.encode(data))).is_err());
        let mut compressed = ZlibEncoder::new(Vec::new(), Compression::best());
        compressed.write_all(&vec![b' '; MAX_JSON + 1]).unwrap();
        let link = format!(
            "{PREFIX}{}",
            URL_SAFE_NO_PAD.encode(compressed.finish().unwrap())
        );
        assert!(parse_share_link(link).is_err());
    }

    #[test]
    fn profile_errors_do_not_expose_the_secret() {
        let mut profile = profile();
        profile.secret = "private-value-do-not-log".into();
        let error = validate_profile(profile.clone()).unwrap_err().to_string();
        assert!(!error.contains(&profile.secret));
        assert!(client_config(&profile).is_err());
        let mut profile = self::profile();
        let mut json: serde_json::Value = serde_json::from_str(&profile.template_json).unwrap();
        json["domain"] = "different.example".into();
        profile.template_json = json.to_string();
        assert!(validate_profile(profile).is_err());
    }
}
