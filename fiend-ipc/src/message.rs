use serde::{Deserialize, Serialize};

use crate::{
    BackupData, IpcError, LogEntry, Profile, ProfileInput, RuleKind, RuleStats, Settings, Stats,
    Status, Subscription,
};

pub const PROTOCOL_MAJOR: u16 = 1;
pub const PROTOCOL_MINOR: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub major: u16,
    pub minor: u16,
    pub client: String,
}

impl Hello {
    pub fn new(client: impl Into<String>) -> Self {
        Self {
            major: PROTOCOL_MAJOR,
            minor: PROTOCOL_MINOR,
            client: client.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloAck {
    pub major: u16,
    pub minor: u16,
    pub daemon_version: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Request {
    Connect {
        profile_id: Option<String>,
    },
    Disconnect,
    Status,
    Subscribe,
    ListProfiles,
    SaveProfile {
        profile: ProfileInput,
    },
    DeleteProfile {
        id: String,
    },
    ImportLink {
        link: String,
    },
    ShareLink {
        id: String,
    },
    GetSettings,
    SetSettings {
        settings: Settings,
    },
    RestoreBackup {
        data: BackupData,
    },
    SetRuleText {
        kind: RuleKind,
        text: String,
    },
    AddSubscription {
        url: String,
        kind: RuleKind,
        name: String,
    },
    UpdateSubscription {
        id: String,
        name: String,
        kind: RuleKind,
    },
    RemoveSubscription {
        id: String,
    },
    RefreshSubscriptions,
    MeasureLatency {
        profile_id: String,
    },
    Logs {
        limit: u32,
    },
    ClearLogs,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Reply {
    Ok,
    Status(Status),
    Profiles { profiles: Vec<Profile> },
    Profile(Profile),
    Link { link: String },
    Settings(Settings),
    Subscription(Subscription),
    RuleStats(RuleStats),
    Latency { ms: u32 },
    Logs { entries: Vec<LogEntry> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    State(Status),
    Stats(Stats),
    Log(LogEntry),
    Settings(Settings),
    ProfilesChanged,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello(Hello),
    Request { id: u64, request: Request },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello(HelloAck),
    Response {
        id: u64,
        result: Result<Reply, IpcError>,
    },
    Event(Event),
}
