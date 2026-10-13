//! File logger for the `log` facade: `%APPDATA%\GamingZones\gamingzones.log`.
//!
//! The level comes from settings.json and can change on reload. Records with target
//! [`DUMP`] are always written, so the debug hotkey works at any level.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

use gz_core::LogLevel;
use log::{Level, LevelFilter, Log, Metadata, Record};

/// Target for output the user asked for explicitly (debug hotkey).
pub const DUMP: &str = "dump";

/// The log is moved to `gamingzones.old.log` at startup once it passes this size.
const ROTATE_BYTES: u64 = 1024 * 1024;

struct FileLogger {
    file: Mutex<File>,
    /// A `LevelFilter` as usize.
    level: AtomicUsize,
}

static LOGGER: OnceLock<FileLogger> = OnceLock::new();

fn filter(level: LogLevel) -> LevelFilter {
    match level {
        LogLevel::Off => LevelFilter::Off,
        LogLevel::Error => LevelFilter::Error,
        LogLevel::Warn => LevelFilter::Warn,
        LogLevel::Info => LevelFilter::Info,
        LogLevel::Debug => LevelFilter::Debug,
        LogLevel::Trace => LevelFilter::Trace,
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.target() == DUMP || metadata.level() as usize <= self.level.load(Ordering::Relaxed)
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let level = match record.level() {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };
        let line = format!("{} {level} {}\r\n", gz_win::time::local_timestamp(), record.args());
        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}

/// Opens the log (rotating it if large) and installs the logger at `Info`.
pub fn init(path: &Path) -> std::io::Result<()> {
    if fs::metadata(path).map(|m| m.len() > ROTATE_BYTES).unwrap_or(false) {
        let _ = fs::rename(path, path.with_extension("old.log"));
    }
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    let logger = LOGGER
        .get_or_init(|| FileLogger { file: Mutex::new(file), level: AtomicUsize::new(LevelFilter::Info as usize) });
    // Filtering happens in `enabled`, so the facade lets everything through.
    let _ = log::set_logger(logger);
    log::set_max_level(LevelFilter::Trace);

    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
    Ok(())
}

pub fn set_level(level: LogLevel) {
    if let Some(logger) = LOGGER.get() {
        logger.level.store(filter(level) as usize, Ordering::Relaxed);
    }
}
