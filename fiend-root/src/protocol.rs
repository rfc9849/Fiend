use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

pub const MAX_ENTRIES: usize = 1024;
pub const FIRST_APP_UID: u32 = 10000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UidRange {
    pub first: u32,
    pub last: u32,
}

/// Which apps go through Fiend. A UID in `exclude` always goes direct.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Policy {
    pub include: Vec<UidRange>,
    pub exclude: Vec<u32>,
    pub ipv6: bool,
    pub bypass_lan: bool,
}

impl Policy {
    pub fn validate(&self) -> Result<(), String> {
        if self.include.is_empty() {
            return Err("No apps are selected".into());
        }
        if self.include.len() + self.exclude.len() > MAX_ENTRIES {
            return Err("Too many apps are selected".into());
        }
        if self
            .include
            .iter()
            .any(|range| range.first > range.last || range.first < FIRST_APP_UID)
        {
            return Err("Only app UIDs can be selected".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    Start { policy: Policy },
    Reply { address: SocketAddr },
}

/// `Ready` carries the TCP listeners and then the UDP sockets. `Socket`
/// carries one UDP reply socket.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Response {
    Ready { tcp: usize, udp: usize },
    Socket,
    Failed { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(include: Vec<UidRange>) -> Policy {
        Policy {
            include,
            exclude: vec![10100],
            ipv6: true,
            bypass_lan: true,
        }
    }

    #[test]
    fn policy_accepts_only_app_uids() {
        assert!(
            policy(vec![UidRange {
                first: 10000,
                last: 19999
            }])
            .validate()
            .is_ok()
        );
        assert!(policy(vec![]).validate().is_err());
        assert!(
            policy(vec![UidRange {
                first: 0,
                last: 19999
            }])
            .validate()
            .is_err()
        );
        assert!(
            policy(vec![UidRange {
                first: 10200,
                last: 10100
            }])
            .validate()
            .is_err()
        );
    }

    #[test]
    fn messages_keep_their_wire_form() {
        let request = Request::Reply {
            address: "192.0.2.1:443".parse().unwrap(),
        };
        let text = serde_json::to_string(&request).unwrap();
        assert_eq!(text, r#"{"op":"reply","address":"192.0.2.1:443"}"#);
        assert_eq!(serde_json::from_str::<Request>(&text).unwrap(), request);
        let text = serde_json::to_string(&Response::Ready { tcp: 2, udp: 2 }).unwrap();
        assert_eq!(text, r#"{"status":"ready","tcp":2,"udp":2}"#);
    }
}
