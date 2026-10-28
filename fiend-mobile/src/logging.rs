use std::fmt::Write;
use std::sync::{Arc, Mutex, Once, Weak};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::{Layer, Registry};

use crate::{LogLine, Platform};

static PLATFORM: Mutex<Option<Weak<dyn Platform>>> = Mutex::new(None);

pub(crate) fn set_platform(platform: &Arc<dyn Platform>) {
    *PLATFORM.lock().unwrap_or_else(|error| error.into_inner()) = Some(Arc::downgrade(platform));
}

pub(crate) fn platform() -> Option<Arc<dyn Platform>> {
    PLATFORM
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .as_ref()
        .and_then(Weak::upgrade)
}

pub(crate) fn initialize(runtime: &Runtime) {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let (sender, mut receiver) = mpsc::channel::<LogLine>(256);
        let subscriber = Registry::default()
            .with(tracing_subscriber::filter::LevelFilter::INFO)
            .with(CallbackLayer(sender));
        #[cfg(target_os = "android")]
        let subscriber = subscriber.with(tracing_android::layer("fiend").ok());
        let _ = tracing::subscriber::set_global_default(subscriber);
        runtime.spawn(async move {
            let mut rate = tokio::time::interval(Duration::from_millis(100));
            rate.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            while let Some(line) = receiver.recv().await {
                rate.tick().await;
                if let Some(platform) = platform() {
                    let _ = platform.on_log(line);
                }
            }
        });
    });
}

struct CallbackLayer(mpsc::Sender<LogLine>);

impl<S: Subscriber> Layer<S> for CallbackLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let mut fields = Fields(String::new());
        event.record(&mut fields);
        let line = LogLine {
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis()
                .min(u128::from(u64::MAX)) as u64,
            level: event.metadata().level().to_string(),
            message: fields.0.chars().take(1024).collect(),
        };
        let _ = self.0.try_send(line);
    }
}

struct Fields(String);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if self.0.len() < 2048 {
            let _ = write!(self.0, "{}={value:?} ", field.name());
        }
    }
}
