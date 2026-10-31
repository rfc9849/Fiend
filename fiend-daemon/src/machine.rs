use fiend_ipc::{ConnectStep, Profile, RuleKind, Stats, TunnelState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EngineSettings {
    pub bypass_lan: bool,
    pub ipv6: bool,
    pub mtu: u16,
    pub direct_resolver: String,
    pub direct_rules: String,
    pub block_rules: String,
    pub proxy_rules: String,
    pub rule_files: Vec<(String, RuleKind, u64)>,
}

#[derive(Clone, Debug)]
pub struct Applied {
    pub profile: Profile,
    pub engine: EngineSettings,
}

#[derive(Debug)]
pub struct Machine {
    state: TunnelState,
    profile_id: Option<String>,
    stats: Option<Stats>,
    applied: Option<Applied>,
}

impl Default for Machine {
    fn default() -> Self {
        Self::new()
    }
}

impl Machine {
    pub fn new() -> Self {
        Self {
            state: TunnelState::Disconnected,
            profile_id: None,
            stats: None,
            applied: None,
        }
    }

    pub fn state(&self) -> &TunnelState {
        &self.state
    }

    pub fn profile_id(&self) -> Option<&str> {
        self.profile_id.as_deref()
    }

    pub fn stats(&self) -> Option<Stats> {
        self.stats
    }

    pub fn applied(&self) -> Option<&Applied> {
        self.applied.as_ref()
    }

    pub fn is_active(&self) -> bool {
        matches!(
            self.state,
            TunnelState::Connecting { .. }
                | TunnelState::Connected { .. }
                | TunnelState::Disconnecting
        )
    }

    pub fn begin_connect(&mut self, applied: Applied) -> bool {
        if self.is_active() {
            return false;
        }
        self.profile_id = Some(applied.profile.id.clone());
        self.applied = Some(applied);
        self.stats = None;
        self.state = TunnelState::Connecting {
            step: ConnectStep::Server,
        };
        true
    }

    pub fn set_step(&mut self, step: ConnectStep) -> bool {
        match self.state {
            TunnelState::Connecting { step: current } if current != step => {
                self.state = TunnelState::Connecting { step };
                true
            }
            _ => false,
        }
    }

    pub fn connected(&mut self, since_ms: u64, latency_ms: Option<u32>) -> bool {
        if !matches!(self.state, TunnelState::Connecting { .. }) {
            return false;
        }
        self.state = TunnelState::Connected { since_ms };
        self.stats = Some(Stats {
            latency_ms,
            ..Stats::default()
        });
        true
    }

    pub fn set_stats(&mut self, stats: Stats) -> bool {
        if matches!(self.state, TunnelState::Connected { .. }) {
            self.stats = Some(stats);
            true
        } else {
            false
        }
    }

    pub fn begin_disconnect(&mut self) -> bool {
        if matches!(
            self.state,
            TunnelState::Connecting { .. } | TunnelState::Connected { .. }
        ) {
            self.state = TunnelState::Disconnecting;
            true
        } else {
            false
        }
    }

    pub fn dismiss(&mut self) -> bool {
        if matches!(self.state, TunnelState::Failed { .. }) {
            self.state = TunnelState::Disconnected;
            self.profile_id = None;
            self.applied = None;
            true
        } else {
            false
        }
    }

    /// End of the session task. `failure` is the error text, if the session
    /// ended by itself with an error.
    pub fn finish(&mut self, failure: Option<String>) -> bool {
        let next = match (&self.state, failure) {
            (TunnelState::Disconnected | TunnelState::Failed { .. }, _) => return false,
            (TunnelState::Disconnecting, _) | (_, None) => TunnelState::Disconnected,
            (_, Some(reason)) => TunnelState::Failed { reason },
        };
        if next == TunnelState::Disconnected {
            self.profile_id = None;
            self.applied = None;
        }
        self.stats = None;
        self.state = next;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn applied() -> Applied {
        Applied {
            profile: Profile {
                id: "p".into(),
                name: "n".into(),
                servers: vec!["127.0.0.1:1".into()],
                secret: String::new(),
                template_json: String::new(),
                tuning: Default::default(),
            },
            engine: EngineSettings {
                bypass_lan: true,
                ipv6: true,
                mtu: 1500,
                direct_resolver: String::new(),
                direct_rules: String::new(),
                block_rules: String::new(),
                proxy_rules: String::new(),
                rule_files: Vec::new(),
            },
        }
    }

    #[test]
    fn normal_cycle() {
        let mut machine = Machine::new();
        assert!(machine.begin_connect(applied()));
        assert_eq!(
            machine.state(),
            &TunnelState::Connecting {
                step: ConnectStep::Server
            }
        );
        assert!(machine.set_step(ConnectStep::Routes));
        assert!(!machine.set_step(ConnectStep::Routes));
        assert!(machine.set_step(ConnectStep::Dns));
        assert!(machine.connected(5, Some(30)));
        assert_eq!(machine.stats().unwrap().latency_ms, Some(30));
        assert!(machine.set_stats(Stats::default()));
        assert!(machine.begin_disconnect());
        assert_eq!(machine.state(), &TunnelState::Disconnecting);
        assert!(machine.finish(None));
        assert_eq!(machine.state(), &TunnelState::Disconnected);
        assert!(machine.profile_id().is_none() && machine.applied().is_none());
    }

    #[test]
    fn connect_is_refused_while_active() {
        let mut machine = Machine::new();
        assert!(machine.begin_connect(applied()));
        assert!(!machine.begin_connect(applied()));
        assert!(machine.connected(1, None));
        assert!(!machine.begin_connect(applied()));
        assert!(machine.begin_disconnect());
        assert!(!machine.begin_connect(applied()));
    }

    #[test]
    fn failure_keeps_the_profile_and_allows_a_new_connect() {
        let mut machine = Machine::new();
        machine.begin_connect(applied());
        assert!(machine.finish(Some("no route".into())));
        assert_eq!(
            machine.state(),
            &TunnelState::Failed {
                reason: "no route".into()
            }
        );
        assert_eq!(machine.profile_id(), Some("p"));
        assert!(!machine.finish(None));
        assert!(machine.dismiss());
        assert_eq!(machine.state(), &TunnelState::Disconnected);
        assert!(machine.profile_id().is_none());
        assert!(!machine.dismiss());
        machine.begin_connect(applied());
        machine.finish(Some("again".into()));
        assert!(machine.begin_connect(applied()));
    }

    #[test]
    fn failure_while_disconnecting_is_a_normal_stop() {
        let mut machine = Machine::new();
        machine.begin_connect(applied());
        machine.connected(1, None);
        machine.begin_disconnect();
        assert!(machine.finish(Some("closed".into())));
        assert_eq!(machine.state(), &TunnelState::Disconnected);
    }

    #[test]
    fn invalid_transitions_do_nothing() {
        let mut machine = Machine::new();
        assert!(!machine.set_step(ConnectStep::Dns));
        assert!(!machine.connected(1, None));
        assert!(!machine.set_stats(Stats::default()));
        assert!(!machine.begin_disconnect());
        assert!(!machine.finish(None));
        machine.begin_connect(applied());
        assert!(!machine.set_stats(Stats::default()));
        assert!(machine.finish(None));
        assert_eq!(machine.state(), &TunnelState::Disconnected);
    }
}
