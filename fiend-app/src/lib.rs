#![forbid(unsafe_code)]

mod error;
mod latency;
mod profile;
mod rules;
#[cfg(feature = "subscription")]
pub mod subscription;

pub use error::AppError;
pub use latency::measure_latency;
pub use profile::{
    LoadedTemplate, ProfileConfig, TemplateInfo, bundled_templates, client_config, load_template,
    make_share_link, parse_share_link, template_domain, validate_profile,
};
pub use rules::{RuleKind, RuleStats, inspect_rule_file, validate_resolver, validate_rules};
