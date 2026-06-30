use chrono::Local;
use log::{Level, Metadata, Record};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

struct FileLogger {
    log_path: PathBuf,
    file_mutex: Mutex<()>,
    /// Level/target filter parsed from `RUST_LOG` (or [`DEFAULT_DIRECTIVES`]).
    /// Lets us mute noisy targets (sqlx statement logging) while keeping our
    /// own DEBUG output, all tunable at runtime without recompiling.
    filter: env_filter::Filter,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.filter.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if self.filter.matches(record) {
            let time_str = Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
            let formatted = format!(
                "[{}] [{}] [{}:{}] {}\n",
                time_str,
                record.level(),
                record.file().unwrap_or("<unknown>"),
                record.line().unwrap_or(0),
                record.args()
            );

            // Print to console
            if record.level() == Level::Error {
                eprint!("{}", formatted);
            } else {
                print!("{}", formatted);
            }

            // Write to file under lock
            let _lock = self.file_mutex.lock().unwrap();
            if let Ok(mut file) = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                let _ = file.write_all(formatted.as_bytes());
            }
        }
    }

    fn flush(&self) {}
}

/// Default log directive used when `RUST_LOG` is unset: keep our own crate at
/// DEBUG, surface everything else at INFO, and mute sqlx's per-statement DEBUG
/// spam (it logs every query under the `sqlx` target) down to WARN so slow-query
/// warnings still come through. Override with `RUST_LOG`, e.g.
/// `RUST_LOG=info,sqlx::query=debug` to bring statement logging back.
const DEFAULT_DIRECTIVES: &str = "info,nebula_lib=debug,sqlx=warn";

/// Build the level/target filter from an optional directive string (typically
/// `RUST_LOG`). When `None`, [`DEFAULT_DIRECTIVES`] is used.
fn build_filter(directives: Option<&str>) -> env_filter::Filter {
    env_filter::Builder::new()
        .parse(directives.unwrap_or(DEFAULT_DIRECTIVES))
        .build()
}

pub fn init(data_dir: &std::path::Path) {
    let log_path = data_dir.join("nebula.log");
    let filter = build_filter(std::env::var("RUST_LOG").ok().as_deref());
    // Set the global max level so `log` can cheaply short-circuit disabled
    // records before they ever reach our `enabled`/`matches` checks.
    let max_level = filter.filter();
    let logger = FileLogger {
        log_path,
        file_mutex: Mutex::new(()),
        filter,
    };
    let _ = log::set_boxed_logger(Box::new(logger)).map(|()| log::set_max_level(max_level));
}

#[cfg(test)]
mod tests {
    use super::build_filter;
    use log::{Level, Metadata};

    fn enabled(f: &env_filter::Filter, level: Level, target: &str) -> bool {
        f.enabled(&Metadata::builder().level(level).target(target).build())
    }

    #[test]
    fn default_keeps_our_debug_but_mutes_sqlx_statement_spam() {
        let f = build_filter(None);
        // Our own crate's debug logs (e.g. the pipeline/sampler) stay visible.
        assert!(
            enabled(&f, Level::Debug, "nebula_lib::pipeline::sampler"),
            "nebula_lib debug logs must remain enabled by default"
        );
        // sqlx logs every statement at DEBUG under the `sqlx` target — muted.
        assert!(
            !enabled(&f, Level::Debug, "sqlx::query"),
            "sqlx per-statement DEBUG spam must be muted by default"
        );
        // ...but sqlx WARN (e.g. slow-query warnings) still comes through.
        assert!(
            enabled(&f, Level::Warn, "sqlx::query"),
            "sqlx warnings must still surface"
        );
        // Unscoped third-party targets default to INFO, not DEBUG.
        assert!(enabled(&f, Level::Info, "tauri::ipc"));
        assert!(!enabled(&f, Level::Debug, "tauri::ipc"));
    }

    #[test]
    fn explicit_directive_overrides_default() {
        // `RUST_LOG=trace` brings everything back, including sqlx statements.
        let f = build_filter(Some("trace"));
        assert!(enabled(&f, Level::Debug, "sqlx::query"));
        assert!(enabled(&f, Level::Trace, "anything::at::all"));
    }
}
