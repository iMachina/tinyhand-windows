//! A small debug log at `%APPDATA%\TinyHand\tinyhand.log`, written only when the
//! `TH_LOG` environment variable is set. Lines also go to the debugger output.

use crate::paths;
use std::io::Write;
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();
static FILE: OnceLock<Option<Mutex<std::fs::File>>> = OnceLock::new();

pub fn write(line: &str) {
    let start = *START.get_or_init(Instant::now);
    let file = FILE.get_or_init(|| {
        std::env::var_os("TH_LOG")?;
        let path = paths::log_file();
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::File::create(path).ok().map(Mutex::new)
    });
    if let Some(file) = file {
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{:9.3} {line}", start.elapsed().as_secs_f64());
        }
    }
}

#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(&format!($($arg)*))
    };
}
