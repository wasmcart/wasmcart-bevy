//! Routes the `log` crate (which Bevy's own crates log through when no
//! tracing subscriber is installed) and panics to the host's `wc_log`.

use log::{Level, LevelFilter, Log, Metadata, Record};

struct WcLogger;

impl Log for WcLogger {
    fn enabled(&self, m: &Metadata) -> bool {
        m.level() <= Level::Info
    }
    fn log(&self, r: &Record) {
        if self.enabled(r.metadata()) {
            let s = format!("[{}] {}: {}", r.level(), r.target(), r.args());
            wasmcart::wc_log(&s);
        }
    }
    fn flush(&self) {}
}

static LOGGER: WcLogger = WcLogger;

pub(crate) fn install() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(LevelFilter::Info);
    }
    std::panic::set_hook(Box::new(|info| {
        wasmcart::wc_log(&format!("panic: {info}"));
    }));
}
