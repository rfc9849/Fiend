use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Config {
    pub max_links: usize,
    pub bulk_lane_bytes: u64,
    pub lane_share: usize,
    pub open_timeout: Duration,
    pub settings_timeout: Duration,
    pub target_timeout: Duration,
    pub ping_interval: Duration,
    pub probe_timeout: Duration,
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
            ping_interval: Duration::from_secs(15),
            probe_timeout: Duration::from_secs(5),
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
