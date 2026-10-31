use crate::FiendError;

#[derive(Clone, uniffi::Record)]
pub struct ProfileConfig {
    pub name: String,
    pub servers: Vec<String>,
    pub secret: String,
    pub template_json: String,
}

#[derive(uniffi::Record)]
pub struct TemplateInfo {
    pub id: String,
    pub name: String,
    pub platform: String,
    pub domain: String,
    pub template_json: String,
}

#[derive(uniffi::Record)]
pub struct LoadedTemplate {
    pub domain: String,
    pub template_json: String,
}

impl From<ProfileConfig> for fiend_app::ProfileConfig {
    fn from(profile: ProfileConfig) -> Self {
        Self {
            name: profile.name,
            servers: profile.servers,
            secret: profile.secret,
            template_json: profile.template_json,
        }
    }
}

impl From<fiend_app::ProfileConfig> for ProfileConfig {
    fn from(profile: fiend_app::ProfileConfig) -> Self {
        Self {
            name: profile.name,
            servers: profile.servers,
            secret: profile.secret,
            template_json: profile.template_json,
        }
    }
}

impl From<fiend_app::TemplateInfo> for TemplateInfo {
    fn from(template: fiend_app::TemplateInfo) -> Self {
        Self {
            id: template.id,
            name: template.name,
            platform: template.platform,
            domain: template.domain,
            template_json: template.template_json,
        }
    }
}

pub(crate) fn client_config(
    profile: &ProfileConfig,
) -> Result<fiend_core::client::ClientConfig, FiendError> {
    Ok(fiend_app::client_config(&profile.clone().into())?)
}

#[uniffi::export]
pub fn validate_profile(profile: ProfileConfig) -> Result<(), FiendError> {
    Ok(fiend_app::validate_profile(profile.into())?)
}

#[uniffi::export]
pub fn make_share_link(profile: ProfileConfig) -> Result<String, FiendError> {
    Ok(fiend_app::make_share_link(profile.into())?)
}

#[uniffi::export]
pub fn parse_share_link(uri: String) -> Result<ProfileConfig, FiendError> {
    Ok(fiend_app::parse_share_link(uri)?.into())
}

#[uniffi::export]
pub fn bundled_templates() -> Vec<TemplateInfo> {
    fiend_app::bundled_templates()
        .into_iter()
        .map(Into::into)
        .collect()
}

#[uniffi::export]
pub fn load_template(data: Vec<u8>) -> Result<LoadedTemplate, FiendError> {
    let loaded = fiend_app::load_template(&data)?;
    Ok(LoadedTemplate {
        domain: loaded.domain,
        template_json: loaded.template_json,
    })
}

#[uniffi::export]
pub fn template_domain(template_json: String) -> Option<String> {
    fiend_app::template_domain(&template_json)
}
