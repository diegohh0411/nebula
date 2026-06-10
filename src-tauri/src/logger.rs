use log::{Record, Level, Metadata, LevelFilter};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use chrono::Local;

struct FileLogger {
    log_path: PathBuf,
    file_mutex: Mutex<()>,
}

impl log::Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Debug
    }

    fn log(&self, record: &Record) {
        if self.enabled(record.metadata()) {
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

pub fn init(data_dir: &std::path::Path) {
    let log_path = data_dir.join("nebula.log");
    let logger = FileLogger {
        log_path,
        file_mutex: Mutex::new(()),
    };
    let _ = log::set_boxed_logger(Box::new(logger))
        .map(|()| log::set_max_level(LevelFilter::Debug));
}
