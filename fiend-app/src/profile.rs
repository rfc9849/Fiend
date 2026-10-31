use fiend_core::client::ClientConfig;
use fiend_core::fingerprint::{Template, parse_client_hello};
use fiend_core::share;
use serde::Deserialize;

use crate::AppError;

const MAX_TEMPLATE: usize = 131072;
const BUNDLED: &str = include_str!("../../fiend-core/testdata/template.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileConfig {
    pub name: String,
    pub servers: Vec<String>,
    pub secret: String,
    pub template_json: String,
}

#[derive(Clone, Debug)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub domain: String,
    pub template_json: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedTemplate {
    pub domain: String,
    pub template_json: String,
}

pub fn client_config(profile: &ProfileConfig) -> Result<ClientConfig, AppError> {
    share::check_name(&profile.name).map_err(share_error)?;
    let servers: Vec<String> = profile
        .servers
        .iter()
        .map(|server| server.trim().to_owned())
        .collect();
    share::check_servers(&servers).map_err(share_error)?;
    let secret = share::decode_secret(&profile.secret).map_err(share_error)?;
    let template = check_template(&profile.template_json)?;
    let mut config = ClientConfig::new(String::new(), secret.to_vec(), template);
    config.servers = servers;
    fiend_core::config::build_client(config).map_err(|error| invalid(error.to_string()))
}

fn check_template(template_json: &str) -> Result<Template, AppError> {
    if template_json.len() > MAX_TEMPLATE {
        return Err(invalid("Template is too large"));
    }
    let template =
        Template::from_json(template_json).map_err(|_| invalid("Template is not valid"))?;
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
    Ok(template)
}

pub fn load_template(data: &[u8]) -> Result<LoadedTemplate, AppError> {
    if data.len() > MAX_TEMPLATE {
        return Err(invalid("Template file is too large"));
    }
    let json = if data.trim_ascii_start().starts_with(b"{") {
        let text = std::str::from_utf8(data).map_err(|_| invalid("Template is not valid"))?;
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|_| invalid("Template is not valid"))?;
        value.to_string()
    } else {
        let template = parse_client_hello(data)
            .map_err(|_| invalid("The file is not a TLS template or a ClientHello capture"))?;
        template
            .verify(data)
            .map_err(|_| invalid("The ClientHello capture cannot be used as a template"))?;
        serde_json::to_string(&template).map_err(|_| invalid("Template is not valid"))?
    };
    let template = check_template(&json)?;
    Ok(LoadedTemplate {
        domain: template.domain,
        template_json: json,
    })
}

pub fn template_domain(template_json: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Domain {
        domain: String,
    }
    serde_json::from_str::<Domain>(template_json)
        .ok()
        .map(|value| value.domain)
}

pub fn validate_profile(profile: ProfileConfig) -> Result<(), AppError> {
    client_config(&profile).map(drop)
}

pub fn parse_servers(text: &str) -> Result<Vec<String>, AppError> {
    share::parse_servers(text).map_err(share_error)
}

pub fn check_servers(servers: &[String]) -> Result<(), AppError> {
    share::check_servers(servers).map_err(share_error)
}

pub fn make_share_link(profile: ProfileConfig) -> Result<String, AppError> {
    let config = client_config(&profile)?;
    let secret = share::decode_secret(&profile.secret).map_err(share_error)?;
    share::encode(&share::Share {
        name: profile.name,
        servers: config.servers,
        secret,
        template: config.template,
    })
    .map_err(share_error)
}

pub fn parse_share_link(uri: String) -> Result<ProfileConfig, AppError> {
    let share = share::decode(&uri).map_err(share_error)?;
    let template_json =
        serde_json::to_string(&share.template).map_err(|_| AppError::InvalidLink)?;
    let profile = ProfileConfig {
        name: share.name,
        servers: share.servers,
        secret: hex::encode(share.secret),
        template_json,
    };
    client_config(&profile)?;
    Ok(profile)
}

pub fn bundled_templates() -> Vec<TemplateInfo> {
    vec![TemplateInfo {
        id: "captured-wikipedia-v1".into(),
        name: "Built-in".into(),
        platform: "TLS".into(),
        domain: template_domain(BUNDLED).unwrap_or_default(),
        template_json: BUNDLED.into(),
    }]
}

fn share_error(error: share::Error) -> AppError {
    match error {
        share::Error::Link => AppError::InvalidLink,
        share::Error::Invalid(reason) => invalid(reason),
    }
}

fn invalid(reason: impl Into<String>) -> AppError {
    AppError::InvalidProfile {
        reason: reason.into(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn profile() -> ProfileConfig {
        ProfileConfig {
            name: "Local test".into(),
            servers: vec!["127.0.0.1:443".into()],
            secret: "42".repeat(32),
            template_json: BUNDLED.into(),
        }
    }

    #[test]
    fn share_link_round_trip_keeps_servers_secret_and_capture() {
        let mut profile = profile();
        profile.servers.push("node.example:8443".into());
        let link = make_share_link(profile.clone()).unwrap();
        assert!(link.starts_with("fiend://") && !link.starts_with("fiend://v1/"));
        let restored = parse_share_link(link.clone()).unwrap();
        assert_eq!(restored.name, profile.name);
        assert_eq!(restored.servers, profile.servers);
        assert_eq!(restored.secret, profile.secret);
        assert_eq!(
            client_config(&restored).unwrap().template.domain,
            client_config(&profile).unwrap().template.domain
        );
        assert_eq!(make_share_link(restored).unwrap(), link);
    }

    #[test]
    fn links_reject_old_formats_and_bad_input() {
        for link in [
            "fiend://v1/eJwrSS0u0U9KzAEAEdMDkg",
            "https://example.com",
            "fiend://!",
            "fiend://AAAA",
        ] {
            assert!(parse_share_link(link.into()).is_err());
        }
    }

    #[test]
    fn templates_load_from_json_or_a_raw_capture() {
        let from_json = load_template(BUNDLED.as_bytes()).unwrap();
        assert_eq!(from_json.domain, "www.wikipedia.org");
        assert_eq!(
            template_domain(&from_json.template_json).as_deref(),
            Some("www.wikipedia.org")
        );
        assert!(from_json.template_json.len() < BUNDLED.len());

        let raw = Template::from_json(BUNDLED).unwrap().raw;
        let from_capture = load_template(&raw).unwrap();
        assert_eq!(from_capture.domain, "www.wikipedia.org");
        assert_eq!(
            Template::from_json(&from_capture.template_json)
                .unwrap()
                .raw,
            raw
        );

        let mut profile = profile();
        profile.template_json = from_capture.template_json;
        let link = make_share_link(profile).unwrap();
        assert_eq!(
            template_domain(&parse_share_link(link).unwrap().template_json).as_deref(),
            Some("www.wikipedia.org")
        );
    }

    #[test]
    fn bad_template_files_are_rejected() {
        let raw = Template::from_json(BUNDLED).unwrap().raw;
        let mut wrong_domain: serde_json::Value = serde_json::from_str(BUNDLED).unwrap();
        wrong_domain["domain"] = "other.example".into();
        for data in [
            Vec::new(),
            b"not a template".to_vec(),
            b"{\"domain\": 1}".to_vec(),
            wrong_domain.to_string().into_bytes(),
            raw[..raw.len() / 2].to_vec(),
            vec![b' '; MAX_TEMPLATE + 1],
        ] {
            assert!(load_template(&data).is_err());
        }
        assert_eq!(template_domain("{}"), None);
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
