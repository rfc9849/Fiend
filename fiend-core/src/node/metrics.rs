use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Metrics {
    pub available: bool,
    pub cpu_percent: f32,
    pub memory_used: u64,
    pub memory_total: u64,
    pub network_rx: u64,
    pub network_tx: u64,
    pub uptime: u64,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
pub struct Sampler {
    system: Option<sysinfo::System>,
    networks: Option<sysinfo::Networks>,
    refreshed: std::time::Instant,
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Default for Sampler {
    fn default() -> Self {
        let mut system = sysinfo::System::new().ok();
        if let Some(system) = &mut system {
            system.refresh_cpu_usage();
            system.refresh_memory();
        }
        Self {
            system,
            networks: sysinfo::Networks::new_with_refreshed_list().ok(),
            refreshed: std::time::Instant::now(),
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
impl Sampler {
    pub fn sample(&mut self) -> Metrics {
        if self.refreshed.elapsed() >= sysinfo::MINIMUM_CPU_UPDATE_INTERVAL {
            if let Some(system) = &mut self.system {
                system.refresh_cpu_usage();
                system.refresh_memory();
            }
            if let Some(networks) = &mut self.networks {
                networks.refresh(true);
            }
            self.refreshed = std::time::Instant::now();
        }
        Metrics {
            available: self.system.is_some(),
            cpu_percent: self
                .system
                .as_ref()
                .map_or(0.0, |system| system.global_cpu_usage()),
            memory_used: self
                .system
                .as_ref()
                .map_or(0, |system| system.used_memory()),
            memory_total: self
                .system
                .as_ref()
                .map_or(0, |system| system.total_memory()),
            network_rx: self.networks.as_ref().map_or(0, |networks| {
                networks
                    .values()
                    .map(|network| network.total_received())
                    .sum()
            }),
            network_tx: self.networks.as_ref().map_or(0, |networks| {
                networks
                    .values()
                    .map(|network| network.total_transmitted())
                    .sum()
            }),
            uptime: sysinfo::System::uptime().unwrap_or_default(),
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
#[derive(Default)]
pub struct Sampler;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
impl Sampler {
    pub fn sample(&mut self) -> Metrics {
        Metrics::default()
    }
}
