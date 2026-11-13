use std::time::Duration;

const PROBE_FLOOR: Duration = Duration::from_secs(3);
const PROBE_CEILING: Duration = Duration::from_secs(20);
const PROBE_UNMEASURED: Duration = Duration::from_secs(5);
const PROBE_RTTS: u32 = 8;
const PROBE_MARGIN: Duration = Duration::from_secs(1);

#[derive(Clone, Debug)]
pub struct Config {
    pub max_links: usize,
    pub bulk_lane_bytes: u64,
    pub lane_share: usize,
    pub open_timeout: Duration,
    pub settings_timeout: Duration,
    pub target_timeout: Duration,
    pub ping_interval: Duration,
    pub probe_timeout: Option<Duration>,
    pub rtt_interval: Option<Duration>,
    pub surplus_idle: Duration,
    pub attempts: usize,
    pub backoff_floor: Duration,
    pub backoff_ceiling: Duration,
    pub stream_window: u32,
    pub connection_window: u32,
    pub max_frame_size: u32,
    pub max_concurrent_streams: u32,
    pub max_send_buffer_size: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_links: 8,
            bulk_lane_bytes: 256 << 10,
            lane_share: 32 << 10,
            open_timeout: Duration::from_secs(45),
            settings_timeout: Duration::from_secs(15),
            target_timeout: Duration::from_secs(15),
            ping_interval: Duration::from_secs(30),
            probe_timeout: None,
            rtt_interval: None,
            surplus_idle: Duration::from_secs(30),
            attempts: 3,
            backoff_floor: Duration::from_millis(250),
            backoff_ceiling: Duration::from_secs(8),
            stream_window: 8 << 20,
            connection_window: 16 << 20,
            max_frame_size: 16 << 10,
            max_concurrent_streams: 200,
            max_send_buffer_size: 64 << 10,
        }
    }
}

pub(crate) fn probe_timeout(fixed: Option<Duration>, srtt: Option<Duration>) -> Duration {
    if let Some(fixed) = fixed {
        return fixed;
    }
    srtt.map_or(PROBE_UNMEASURED, |srtt| {
        (srtt.saturating_mul(PROBE_RTTS) + PROBE_MARGIN).clamp(PROBE_FLOOR, PROBE_CEILING)
    })
}

impl Config {
    pub(crate) fn ping_tick(&self) -> Duration {
        self.rtt_interval
            .map_or(self.ping_interval, |rtt| rtt.min(self.ping_interval))
    }

    pub(crate) fn rtt_max_age(&self) -> Duration {
        match self.rtt_interval {
            Some(_) => self.ping_tick() * 2 + self.probe_timeout.unwrap_or(PROBE_CEILING),
            None => Duration::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_probe_timeout_follows_the_rtt_unless_it_is_set() {
        assert_eq!(probe_timeout(None, None), PROBE_UNMEASURED);
        assert_eq!(
            probe_timeout(None, Some(Duration::from_millis(20))),
            PROBE_FLOOR
        );
        assert_eq!(
            probe_timeout(None, Some(Duration::from_millis(600))),
            Duration::from_millis(5800)
        );
        assert_eq!(
            probe_timeout(None, Some(Duration::from_secs(10))),
            PROBE_CEILING
        );
        assert_eq!(
            probe_timeout(
                Some(Duration::from_secs(7)),
                Some(Duration::from_millis(600))
            ),
            Duration::from_secs(7)
        );
    }
}
