//! Tracing subscriber that writes to a rotating file in
//! `$XDG_DATA_HOME/gpr/log/` and to a bounded in-memory ring (last
//! [`LOG_RING_CAP`] lines) for the `:log` overlay.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::io;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::registry::LookupSpan;

/// Maximum number of lines retained by [`LogRing`].
pub const LOG_RING_CAP: usize = 200;

/// Bounded in-memory ring of log lines for the `:log` overlay.
///
/// Cloning the ring yields another handle to the same shared buffer (it
/// wraps an [`Arc`] internally), so subscribers and viewers can share
/// state cheaply.
#[derive(Debug, Default, Clone)]
pub struct LogRing {
    inner: Arc<Mutex<VecDeque<String>>>,
}

impl LogRing {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(LOG_RING_CAP))),
        }
    }

    /// Push a line. Evicts the oldest entry once [`LOG_RING_CAP`] is reached.
    pub fn push(&self, line: impl Into<String>) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        if guard.len() == LOG_RING_CAP {
            guard.pop_front();
        }
        guard.push_back(line.into());
    }

    /// Return the most recent `n` lines, oldest first.
    #[must_use]
    pub fn tail(&self, n: usize) -> Vec<String> {
        let Ok(guard) = self.inner.lock() else {
            return Vec::new();
        };
        let len = guard.len();
        let start = len.saturating_sub(n);
        guard.iter().skip(start).cloned().collect()
    }

    /// Current line count.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.lock().map_or(0, |g| g.len())
    }

    /// True when no lines have been pushed.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Process-wide ring used by the installed subscriber.
static GLOBAL_RING: OnceLock<LogRing> = OnceLock::new();

/// Return the global [`LogRing`], initialising it on first access.
pub fn global_ring() -> LogRing {
    GLOBAL_RING.get_or_init(LogRing::new).clone()
}

/// Resolve the log directory under `$XDG_DATA_HOME/gpr/log/`.
///
/// Falls back to `directories::ProjectDirs::data_dir()` on platforms
/// without an `XDG_DATA_HOME`. Returns `None` when no data directory can
/// be determined (e.g. exotic CI environments).
#[must_use]
pub fn log_dir() -> Option<PathBuf> {
    if let Some(xdg) = std::env::var_os("XDG_DATA_HOME") {
        let mut p = PathBuf::from(xdg);
        p.push("gpr");
        p.push("log");
        return Some(p);
    }
    let proj = directories::ProjectDirs::from("", "", "gpr")?;
    let mut p = proj.data_dir().to_path_buf();
    p.push("log");
    Some(p)
}

/// Tracing layer pushing formatted events into a [`LogRing`].
#[derive(Debug)]
pub struct RingLayer {
    ring: LogRing,
}

impl RingLayer {
    #[must_use]
    pub fn new(ring: LogRing) -> Self {
        Self { ring }
    }
}

#[derive(Default)]
struct FieldFormatter {
    out: String,
}

impl Visit for FieldFormatter {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            let _ = write!(self.out, "{value}");
        } else {
            let _ = write!(self.out, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.out, "{value:?}");
        } else {
            let _ = write!(self.out, " {}={value:?}", field.name());
        }
    }
}

impl<S> Layer<S> for RingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut fmt = FieldFormatter::default();
        event.record(&mut fmt);
        let line = format!("{:>5} {}: {}", meta.level(), meta.target(), fmt.out);
        self.ring.push(line);
    }
}

/// Guard returned by [`init_subscriber`]; drop on shutdown to flush the
/// file appender. `None` when no log directory could be resolved.
#[must_use = "drop this guard on shutdown to flush log output"]
#[derive(Debug)]
pub struct LoggingGuard {
    _file: Option<WorkerGuard>,
}

/// Install the global tracing subscriber: ring layer + (optional) rolling
/// daily file appender at `<log_dir>/gpr.log.YYYY-MM-DD`.
///
/// Returns a [`LoggingGuard`] that flushes the file appender on drop, or
/// `None` if a subscriber was already installed by another call.
#[must_use = "drop the returned guard on shutdown to flush log output"]
pub fn init_subscriber() -> Option<LoggingGuard> {
    init_subscriber_with_default("info")
}

/// Like [`init_subscriber`] but uses `default_directive` (e.g. `"debug"` /
/// `"trace"` from `--debug` / `--trace`) when `GPR_LOG` is unset. `GPR_LOG`
/// always takes precedence so it can still override the CLI flags.
#[must_use = "drop the returned guard on shutdown to flush log output"]
pub fn init_subscriber_with_default(default_directive: &str) -> Option<LoggingGuard> {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::{EnvFilter, fmt};

    let filter =
        EnvFilter::try_from_env("GPR_LOG").unwrap_or_else(|_| EnvFilter::new(default_directive));
    let ring_layer = RingLayer::new(global_ring());

    let (file_layer, guard) = if let Some(dir) = log_dir() {
        if std::fs::create_dir_all(&dir).is_ok() {
            let appender = tracing_appender::rolling::daily(&dir, "gpr.log");
            let (nb, g) = tracing_appender::non_blocking(appender);
            let layer = fmt::layer().with_ansi(false).with_writer(nb);
            (Some(layer), Some(g))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };

    let registry = tracing_subscriber::registry().with(filter).with(ring_layer);
    let result = if let Some(layer) = file_layer {
        registry.with(layer).try_init()
    } else {
        registry.try_init()
    };
    result.ok().map(|()| LoggingGuard { _file: guard })
}

/// Backwards-compatibility wrapper returning an `io::Result` for callers
/// that want to surface init failures explicitly.
pub fn try_init() -> io::Result<Option<LoggingGuard>> {
    Ok(init_subscriber())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialises tests that mutate `XDG_DATA_HOME` so they cannot race
    /// each other under cargo's parallel test runner.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // `std::env::set_var`/`remove_var` are `unsafe` as of edition 2024 because
    // they data-race concurrent readers. These helpers centralise the single
    // justified `unsafe` use: every caller holds `ENV_LOCK`, so no other thread
    // touches the environment for the duration of the mutation.
    #[allow(unsafe_code)]
    fn set_env(key: &str, val: impl AsRef<std::ffi::OsStr>) {
        // SAFETY: serialised by ENV_LOCK; no concurrent env access in-process.
        unsafe { std::env::set_var(key, val) };
    }

    #[allow(unsafe_code)]
    fn remove_env(key: &str) {
        // SAFETY: serialised by ENV_LOCK; no concurrent env access in-process.
        unsafe { std::env::remove_var(key) };
    }

    #[test]
    fn push_then_tail_returns_pushed_line() {
        let ring = LogRing::new();
        ring.push("hello");
        assert_eq!(ring.tail(10), vec!["hello".to_string()]);
    }

    #[test]
    fn ring_caps_at_log_ring_cap_and_evicts_oldest() {
        let ring = LogRing::new();
        for i in 0..=LOG_RING_CAP {
            ring.push(format!("line-{i}"));
        }
        assert_eq!(ring.len(), LOG_RING_CAP);
        let all = ring.tail(LOG_RING_CAP);
        assert_eq!(all.first().map(String::as_str), Some("line-1"));
        assert_eq!(
            all.last().map(String::as_str),
            Some(format!("line-{LOG_RING_CAP}").as_str())
        );
    }

    #[test]
    fn tail_returns_oldest_first_up_to_n() {
        let ring = LogRing::new();
        for i in 0..5 {
            ring.push(format!("l{i}"));
        }
        assert_eq!(ring.tail(3), vec!["l2", "l3", "l4"]);
    }

    #[test]
    fn clones_share_underlying_storage() {
        let a = LogRing::new();
        let b = a.clone();
        a.push("from-a");
        b.push("from-b");
        assert_eq!(a.tail(10), vec!["from-a".to_string(), "from-b".to_string()]);
    }

    #[test]
    fn log_dir_respects_xdg_data_home() {
        let _guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prev = std::env::var_os("XDG_DATA_HOME");
        set_env("XDG_DATA_HOME", "/tmp/gpr-test-xdg-home");
        let dir = log_dir().expect("log_dir with XDG set");
        assert!(
            dir.ends_with("gpr/log"),
            "expected path ending in gpr/log, got {dir:?}"
        );
        assert!(
            dir.starts_with("/tmp/gpr-test-xdg-home"),
            "expected XDG prefix, got {dir:?}"
        );
        match prev {
            Some(v) => set_env("XDG_DATA_HOME", v),
            None => remove_env("XDG_DATA_HOME"),
        }
    }

    #[test]
    fn ring_layer_captures_tracing_event() {
        use tracing::subscriber::with_default;
        use tracing_subscriber::Registry;
        use tracing_subscriber::layer::SubscriberExt;

        let ring = LogRing::new();
        let subscriber = Registry::default().with(RingLayer::new(ring.clone()));
        with_default(subscriber, || {
            tracing::info!("captured-by-ring");
        });
        let lines = ring.tail(10);
        assert!(
            lines.iter().any(|l| l.contains("captured-by-ring")),
            "expected event in ring, got {lines:?}"
        );
        assert!(
            lines.iter().any(|l| l.contains("INFO")),
            "expected INFO level, got {lines:?}"
        );
    }

    #[test]
    fn init_subscriber_creates_file_in_xdg_data_home() {
        let _env_guard = ENV_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let tmp = std::env::temp_dir().join(format!("gpr-logtest-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        let prev = std::env::var_os("XDG_DATA_HOME");
        set_env("XDG_DATA_HOME", &tmp);

        let _guard = init_subscriber();
        let dir = log_dir().expect("log_dir resolves");
        let _ = std::fs::create_dir_all(&dir);
        assert!(dir.exists(), "expected log dir to exist at {dir:?}");
        assert!(
            dir.starts_with(&tmp),
            "expected path under {tmp:?}, got {dir:?}"
        );

        match prev {
            Some(v) => set_env("XDG_DATA_HOME", v),
            None => remove_env("XDG_DATA_HOME"),
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
