use std::collections::{HashMap, VecDeque};

use fiend_ipc::{LogEntry, Profile, Settings, Stats, Status, TunnelState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServiceState {
    Starting,
    Ready,
    NotInstalled,
    NotRunning,
    Refused(String),
    Incompatible(String),
}

impl ServiceState {
    pub fn is_ready(&self) -> bool {
        matches!(self, Self::Ready)
    }
}

#[derive(Clone, Debug)]
pub enum Update {
    Service(ServiceState),
    Status(Status),
    Profiles(Vec<Profile>),
    Settings(Settings),
    Logs(Vec<LogEntry>),
    Log(LogEntry),
    Stats(Stats),
}

pub const LOG_LIMIT: usize = 1000;
pub const HISTORY_LIMIT: usize = 60;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample {
    pub download_bps: u64,
    pub upload_bps: u64,
    pub latency_ms: Option<u32>,
}

impl From<Stats> for Sample {
    fn from(stats: Stats) -> Self {
        Self {
            download_bps: stats.download_bps,
            upload_bps: stats.upload_bps,
            latency_ms: stats.latency_ms,
        }
    }
}

pub struct Model {
    pub service: ServiceState,
    pub status: Status,
    pub profiles: Vec<Profile>,
    pub settings: Settings,
    pub logs: Vec<LogEntry>,
    pub latencies: HashMap<String, u32>,
    pub history: VecDeque<Sample>,
}

impl Default for Model {
    fn default() -> Self {
        Self {
            service: ServiceState::Starting,
            status: Status {
                state: TunnelState::Disconnected,
                profile_id: None,
                stats: None,
                restart_required: false,
                daemon_version: String::new(),
            },
            profiles: Vec::new(),
            settings: Settings::default(),
            logs: Vec::new(),
            latencies: HashMap::new(),
            history: VecDeque::with_capacity(HISTORY_LIMIT),
        }
    }
}

impl Model {
    pub fn apply(&mut self, update: Update) {
        match update {
            Update::Service(service) => {
                if !service.is_ready() {
                    self.status.state = TunnelState::Disconnected;
                    self.status.stats = None;
                    self.status.restart_required = false;
                    self.history.clear();
                }
                self.service = service;
            }
            Update::Status(status) => {
                let same_session = matches!(
                    (&self.status.state, &status.state),
                    (
                        TunnelState::Connected { since_ms: before },
                        TunnelState::Connected { since_ms: after },
                    ) if before == after
                ) && self.status.profile_id == status.profile_id;
                if !same_session {
                    self.history.clear();
                }
                self.status = status;
            }
            Update::Profiles(profiles) => {
                self.latencies
                    .retain(|id, _| profiles.iter().any(|profile| &profile.id == id));
                self.profiles = profiles;
            }
            Update::Settings(settings) => self.settings = settings,
            Update::Logs(logs) => self.logs = logs,
            Update::Log(entry) => {
                self.logs.push(entry);
                if self.logs.len() > LOG_LIMIT {
                    self.logs.remove(0);
                }
            }
            Update::Stats(stats) => {
                if self.history.len() == HISTORY_LIMIT {
                    self.history.pop_front();
                }
                self.history.push_back(stats.into());
                self.status.stats = Some(stats);
            }
        }
    }

    pub fn profile(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|profile| profile.id == id)
    }

    pub fn active_profile(&self) -> Option<&Profile> {
        self.status
            .profile_id
            .as_deref()
            .or(self.settings.active_profile.as_deref())
            .and_then(|id| self.profile(id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fiend_ipc::LogLevel;

    fn entry(message: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 0,
            level: LogLevel::Info,
            message: message.into(),
        }
    }

    #[test]
    fn logs_keep_the_newest_entries() {
        let mut model = Model::default();
        for index in 0..LOG_LIMIT + 3 {
            model.apply(Update::Log(entry(&index.to_string())));
        }
        assert_eq!(model.logs.len(), LOG_LIMIT);
        assert_eq!(
            model.logs.last().unwrap().message,
            (LOG_LIMIT + 2).to_string()
        );
        assert_eq!(model.logs[0].message, "3");
    }

    #[test]
    fn losing_the_service_resets_the_tunnel_view() {
        let mut model = Model::default();
        model.apply(Update::Stats(Stats::default()));
        model.status.state = TunnelState::Connected { since_ms: 1 };
        model.apply(Update::Service(ServiceState::NotRunning));
        assert_eq!(model.status.state, TunnelState::Disconnected);
        assert!(model.status.stats.is_none());
        assert!(model.history.is_empty());
    }

    fn connected(since_ms: u64) -> Status {
        Status {
            state: TunnelState::Connected { since_ms },
            profile_id: Some("a".into()),
            stats: None,
            restart_required: false,
            daemon_version: String::new(),
        }
    }

    #[test]
    fn history_keeps_one_session_and_the_newest_samples() {
        let mut model = Model::default();
        model.apply(Update::Status(connected(1)));
        for index in 0..HISTORY_LIMIT as u64 + 5 {
            model.apply(Update::Stats(Stats {
                download_bps: index,
                ..Stats::default()
            }));
        }
        assert_eq!(model.history.len(), HISTORY_LIMIT);
        assert_eq!(model.history[0].download_bps, 5);
        model.apply(Update::Status(connected(1)));
        assert_eq!(model.history.len(), HISTORY_LIMIT);
        model.apply(Update::Status(connected(2)));
        assert!(model.history.is_empty());
    }
}
