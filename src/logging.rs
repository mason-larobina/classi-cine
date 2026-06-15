use std::fs::File;
use std::io::{self, Write};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

/// When true, the TUI owns the terminal (alternate screen) and stderr logging
/// is suppressed so it doesn't corrupt the display. File logging continues.
static TUI_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Mark whether the TUI currently owns the terminal. While active, log records
/// are no longer written to stderr (but are still written to the log file, if
/// one was configured).
pub fn set_tui_active(active: bool) {
    TUI_ACTIVE.store(active, Ordering::SeqCst);
}

fn tui_active() -> bool {
    TUI_ACTIVE.load(Ordering::SeqCst)
}

/// env_logger sink that always writes to an optional log file and writes to
/// stderr only while the TUI is not in the alternate screen.
struct LogSink {
    file: Option<Mutex<File>>,
}

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(file) = &self.file
            && let Ok(mut f) = file.lock()
        {
            let _ = f.write_all(buf);
        }
        if !tui_active() {
            let _ = io::stderr().write_all(buf);
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(file) = &self.file
            && let Ok(mut f) = file.lock()
        {
            let _ = f.flush();
        }
        if !tui_active() {
            let _ = io::stderr().flush();
        }
        Ok(())
    }
}

/// Initialize logging. `log_level` is the default filter (overridable via the
/// `RUST_LOG` environment variable). When `log_file` is set, all log output is
/// also written to that file, regardless of TUI state.
pub fn init(log_level: &str, log_file: Option<&Path>) -> io::Result<()> {
    let file = match log_file {
        Some(path) => Some(Mutex::new(File::create(path)?)),
        None => None,
    };

    let filters = std::env::var("RUST_LOG").unwrap_or_else(|_| log_level.to_string());

    env_logger::Builder::new()
        .parse_filters(&filters)
        .target(env_logger::Target::Pipe(Box::new(LogSink { file })))
        .init();

    Ok(())
}
