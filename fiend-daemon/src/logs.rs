use std::collections::VecDeque;
use std::fmt::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fiend_ipc::{LogEntry, LogLevel};
use tokio::sync::broadcast;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

const CAPACITY: usize = 2000;
const WINDOW: Duration = Duration::from_millis(100);
const WINDOW_LINES: u32 = 20;

pub struct LogBuffer {
    entries: Mutex<VecDeque<LogEntry>>,
    sender: broadcast::Sender<LogEntry>,
}

impl Default for LogBuffer {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuffer {
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(VecDeque::new()),
            sender: broadcast::channel(256).0,
        }
    }

    pub fn push(&self, entry: LogEntry) {
        {
            let mut entries = self
                .entries
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if entries.len() == CAPACITY {
                entries.pop_front();
            }
            entries.push_back(entry.clone());
        }
        let _ = self.sender.send(entry);
    }

    pub fn recent(&self, limit: usize) -> Vec<LogEntry> {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let skip = entries.len().saturating_sub(limit);
        entries.iter().skip(skip).cloned().collect()
    }

    pub fn clear(&self) {
        self.entries
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clear();
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogEntry> {
        self.sender.subscribe()
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u128::from(u64::MAX)) as u64
}

struct BufferLayer {
    buffer: Arc<LogBuffer>,
    window: Mutex<(Instant, u32)>,
}

impl BufferLayer {
    fn admit(&self, level: Level) -> bool {
        if level <= Level::WARN {
            return true;
        }
        let mut window = self
            .window
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if window.0.elapsed() >= WINDOW {
            *window = (Instant::now(), 0);
        }
        window.1 += 1;
        window.1 <= WINDOW_LINES
    }
}

impl<S: Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let level = *event.metadata().level();
        if !self.admit(level) {
            return;
        }
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        self.buffer.push(LogEntry {
            timestamp_ms: now_ms(),
            level: match level {
                Level::ERROR => LogLevel::Error,
                Level::WARN => LogLevel::Warn,
                Level::INFO => LogLevel::Info,
                Level::DEBUG => LogLevel::Debug,
                Level::TRACE => LogLevel::Trace,
            },
            message: fields.0.trim_end().chars().take(1024).collect(),
        });
    }
}

struct Fields(String);

impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if self.0.len() >= 2048 {
            return;
        }
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?} ");
        } else {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}

pub fn install_logging() -> Arc<LogBuffer> {
    let buffer = Arc::new(LogBuffer::new());
    let subscriber = Registry::default()
        .with(tracing_subscriber::filter::LevelFilter::INFO)
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(BufferLayer {
            buffer: buffer.clone(),
            window: Mutex::new((Instant::now(), 0)),
        });
    let _ = tracing::subscriber::set_global_default(subscriber);
    buffer
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(message: &str) -> LogEntry {
        LogEntry {
            timestamp_ms: 1,
            level: LogLevel::Info,
            message: message.into(),
        }
    }

    #[test]
    fn buffer_keeps_the_newest_entries() {
        let buffer = LogBuffer::new();
        for index in 0..CAPACITY + 5 {
            buffer.push(entry(&index.to_string()));
        }
        let recent = buffer.recent(3);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[2].message, (CAPACITY + 4).to_string());
        assert_eq!(buffer.recent(usize::MAX).len(), CAPACITY);
        buffer.clear();
        assert!(buffer.recent(10).is_empty());
    }

    #[test]
    fn layer_limits_info_but_not_warnings() {
        let buffer = Arc::new(LogBuffer::new());
        let subscriber = Registry::default().with(BufferLayer {
            buffer: buffer.clone(),
            window: Mutex::new((Instant::now(), 0)),
        });
        tracing::subscriber::with_default(subscriber, || {
            for index in 0..100 {
                tracing::info!(index, "burst");
            }
            for _ in 0..30 {
                tracing::warn!("problem");
            }
        });
        let entries = buffer.recent(usize::MAX);
        let infos = entries.iter().filter(|e| e.level == LogLevel::Info).count();
        let warns = entries.iter().filter(|e| e.level == LogLevel::Warn).count();
        assert_eq!(infos, WINDOW_LINES as usize);
        assert_eq!(warns, 30);
        assert!(entries[0].message.starts_with("burst"));
    }
}
