use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct ErrorBody {
    pub error: String,
    pub field: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct SetupStatus {
    pub open: bool,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct TotpSetup {
    pub secret: String,
    pub uri: String,
    pub qr_svg: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct SetupAdmin {
    pub token: String,
    pub name: String,
    pub password: String,
    pub totp_secret: Option<String>,
    pub totp_code: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct Login {
    pub name: String,
    pub password: String,
    pub totp_code: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct AdminView {
    pub id: i64,
    pub name: String,
    pub totp: bool,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct SessionInfo {
    pub admin: AdminView,
    pub csrf: String,
    pub wizard: WizardState,
    pub version: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct WizardState {
    pub local_node: bool,
    pub disguise: bool,
    pub first_user: bool,
    pub done: bool,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct LocalNodeDefaults {
    pub available: bool,
    pub imported: bool,
    pub port: u16,
    pub origin_host: String,
    pub origin_port: u16,
    pub users: usize,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct LocalNodeInput {
    pub name: String,
    pub address: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct NodeView {
    pub id: i64,
    pub name: String,
    pub address: String,
    pub port: u16,
    pub local: bool,
    pub arch: Option<String>,
    pub version: Option<String>,
    pub binary_sha256: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
    pub last_seen: Option<i64>,
    pub started_at: Option<i64>,
    pub cpu: Option<f64>,
    pub memory_used: Option<i64>,
    pub memory_total: Option<i64>,
    pub traffic_rate: f64,
    pub applied_version: i64,
    pub desired_version: i64,
    pub disguise_id: Option<i64>,
    pub users: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct NodeInput {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub disguise_id: Option<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct SshScan {
    pub host: String,
    pub port: u16,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct HostKeyView {
    pub scan_id: String,
    pub fingerprint: String,
    pub algorithm: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct SshInstall {
    pub scan_id: String,
    pub name: String,
    pub address: String,
    pub ssh_port: u16,
    pub user: String,
    pub private_key: Option<String>,
    pub passphrase: Option<String>,
    pub password: Option<String>,
    pub sudo_password: Option<String>,
    pub node_port: u16,
    pub disguise_id: Option<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct JobStep {
    pub name: String,
    pub state: String,
    pub detail: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct JobView {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub steps: Vec<JobStep>,
    pub done: bool,
    pub error: Option<String>,
    pub node_id: Option<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct NodeLogs {
    pub lines: Vec<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct NodeUpdate {
    pub node_ids: Vec<i64>,
    pub bundle_id: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct WarpView {
    pub enabled: bool,
    pub registered: bool,
    pub running: bool,
    pub exit_v4: Option<String>,
    pub exit_v6: Option<String>,
    pub colo: Option<String>,
    pub warp_on: bool,
    pub created_at: Option<i64>,
    pub rotated_at: Option<i64>,
    pub rotate_every: Option<i64>,
    pub error: Option<String>,
    pub checked_at: Option<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct WarpSettingsInput {
    pub enabled: bool,
    pub rotate_every: Option<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct WarpRotation {
    pub before_v4: Option<String>,
    pub before_v6: Option<String>,
    pub after_v4: Option<String>,
    pub after_v6: Option<String>,
    pub warp: WarpView,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct UserView {
    pub id: i64,
    pub name: String,
    pub enabled: bool,
    pub state: String,
    pub note: String,
    pub traffic_limit: Option<i64>,
    pub used: i64,
    pub expire_at: Option<i64>,
    pub hold_days: Option<i64>,
    pub reset_period: String,
    pub reset_at: Option<i64>,
    pub last_seen: Option<i64>,
    pub online: i64,
    pub nodes: Vec<i64>,
    pub created_at: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct UserInput {
    pub name: String,
    pub enabled: bool,
    pub note: String,
    pub traffic_limit: Option<i64>,
    pub expire_at: Option<i64>,
    pub hold_days: Option<i64>,
    pub reset_period: String,
    pub nodes: Vec<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct UserLink {
    pub link: String,
    pub qr_svg: String,
    pub servers: Vec<String>,
    pub warnings: Vec<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct UserSecret {
    pub secret: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct NodeUsage {
    pub node_id: i64,
    pub node_name: String,
    pub up: i64,
    pub down: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct NodeSessions {
    pub node_id: i64,
    pub node_name: String,
    pub started_at: Vec<i64>,
    pub error: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct UserDetail {
    pub user: UserView,
    pub usage: Vec<NodeUsage>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct OriginProfileView {
    pub cipher_suite: u16,
    pub key_share_group: u16,
    pub extension_order: Vec<u16>,
    pub record_sizes: Vec<u16>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct DisguiseView {
    pub id: i64,
    pub name: String,
    pub origin_host: String,
    pub origin_port: u16,
    pub domain: String,
    pub template_size: i64,
    pub profile: Option<OriginProfileView>,
    pub probed_at: Option<i64>,
    pub probe_error: Option<String>,
    pub nodes: i64,
    pub checks: Vec<Check>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct DisguiseInput {
    pub name: String,
    pub origin_host: String,
    pub origin_port: u16,
    pub preset: Option<String>,
    pub template_base64: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct PresetView {
    pub id: String,
    pub name: String,
    pub domain: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct ProbeInput {
    pub node_id: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct OutboundView {
    pub id: i64,
    pub node_id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub server: Option<String>,
    pub username: Option<String>,
    pub has_password: bool,
    pub udp: bool,
    pub link_node_id: Option<i64>,
    pub udp_fallback: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct OutboundInput {
    pub node_id: Option<i64>,
    pub name: String,
    pub kind: String,
    pub server: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    pub udp: bool,
    pub link_node_id: Option<i64>,
    pub udp_fallback: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct RuleView {
    pub id: i64,
    pub node_id: Option<i64>,
    pub position: i64,
    pub pattern: String,
    pub outbound: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct RuleInput {
    pub node_id: Option<i64>,
    pub pattern: String,
    pub outbound: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct RuleOrder {
    pub ids: Vec<i64>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct Routing {
    pub outbounds: Vec<OutboundView>,
    pub rules: Vec<RuleView>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct AdminInput {
    pub name: String,
    pub password: String,
    pub current_code: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct PasswordInput {
    pub current: String,
    pub password: String,
    pub totp_code: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct TotpInput {
    pub secret: String,
    pub code: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct CodeInput {
    pub code: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct BundleView {
    pub id: i64,
    pub version: String,
    pub arch: String,
    pub sha256: String,
    pub core_sha256: String,
    pub uploaded_at: i64,
    pub size: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct AuditView {
    pub id: i64,
    pub ts: i64,
    pub admin: Option<String>,
    pub action: String,
    pub target: String,
    pub detail: String,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct TrafficPoint {
    pub ts: i64,
    pub up: i64,
    pub down: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct TrafficSeries {
    pub range: String,
    pub step: i64,
    pub points: Vec<TrafficPoint>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct Dashboard {
    pub traffic_rate: f64,
    pub users_online: i64,
    pub users_total: i64,
    pub nodes_online: i64,
    pub nodes_total: i64,
    pub origin_recent: i64,
    pub traffic_samples: Vec<f64>,
    pub online_samples: Vec<f64>,
    pub node_samples: Vec<f64>,
    pub origin_samples: Vec<f64>,
    pub nodes: Vec<NodeView>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct Event {
    pub kind: String,
    pub id: Option<String>,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct Created {
    pub id: i64,
}

#[derive(Serialize, Deserialize, TS, Clone, Debug)]
pub struct JobStarted {
    pub job_id: String,
}

pub fn export(dir: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let config = ts_rs::Config::new()
        .with_large_int("number")
        .with_out_dir(dir);
    macro_rules! all {
        ($($name:ty),* $(,)?) => { $( <$name as TS>::export_all(&config)?; )* };
    }
    all!(
        ErrorBody,
        SetupStatus,
        TotpSetup,
        SetupAdmin,
        Login,
        AdminView,
        SessionInfo,
        WizardState,
        LocalNodeDefaults,
        LocalNodeInput,
        NodeView,
        NodeInput,
        SshScan,
        HostKeyView,
        SshInstall,
        JobStep,
        JobView,
        NodeLogs,
        NodeUpdate,
        WarpView,
        WarpSettingsInput,
        WarpRotation,
        UserView,
        UserInput,
        UserLink,
        UserSecret,
        NodeUsage,
        NodeSessions,
        UserDetail,
        Check,
        OriginProfileView,
        DisguiseView,
        DisguiseInput,
        PresetView,
        ProbeInput,
        OutboundView,
        OutboundInput,
        RuleView,
        RuleInput,
        RuleOrder,
        Routing,
        AdminInput,
        PasswordInput,
        TotpInput,
        CodeInput,
        BundleView,
        AuditView,
        TrafficPoint,
        TrafficSeries,
        Dashboard,
        Event,
        Created,
        JobStarted,
    );
    let mut names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter_map(|name| name.strip_suffix(".ts").map(str::to_owned))
        .filter(|name| name != "index")
        .collect();
    names.sort();
    let mut index =
        String::from("// Generated by fiend-panel export-types. Do not edit this file manually.\n\n");
    for name in names {
        index.push_str(&format!("export type {{ {name} }} from './{name}';\n"));
    }
    std::fs::write(dir.join("index.ts"), index)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn generated_types_match_the_rust_api() {
        let committed =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("web/src/lib/api/generated");
        let fresh = std::env::temp_dir().join(format!("fiend-types-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&fresh);
        std::fs::create_dir_all(&fresh).unwrap();
        super::export(&fresh).unwrap();
        let mut names: Vec<_> = std::fs::read_dir(&fresh)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        names.sort();
        for name in names {
            let expected = std::fs::read_to_string(fresh.join(&name)).unwrap();
            let actual = std::fs::read_to_string(committed.join(&name)).unwrap_or_default();
            assert_eq!(
                actual, expected,
                "run: cargo run -p fiend-panel -- export-types fiend-panel/web/src/lib/api/generated ({name:?})"
            );
        }
        std::fs::remove_dir_all(fresh).unwrap();
    }
}
