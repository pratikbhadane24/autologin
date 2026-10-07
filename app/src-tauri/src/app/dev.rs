//! Development-only switches for exercising the real app without personal
//! data or clicks. All are ignored in release builds.
//!
//! - `AUTOLOGIN_DEV_DATA_DIR=<dir>`: separate data + logs folder; also runs as
//!   a separate instance (no single-instance hand-off).
//! - `AUTOLOGIN_DEV_MEMORY_SECRETS=1`: keep secrets in memory (no keychain).
//! - `AUTOLOGIN_DEV_RUN_ALL=1`: start "Log in all accounts" a few seconds
//!   after launch (visible browser), then quit when it finishes.
//! - `AUTOLOGIN_DEV_CHROME_LOG=<file>`: copy Chrome's own debug log there.

use std::path::PathBuf;

#[derive(Debug, Clone, Default)]
pub struct DevOptions {
    pub data_dir: Option<PathBuf>,
    pub memory_secrets: bool,
    pub run_all_then_quit: bool,
}

fn flag(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| v == "1")
}

pub fn options() -> DevOptions {
    if !cfg!(debug_assertions) {
        return DevOptions::default();
    }
    DevOptions {
        data_dir: std::env::var_os("AUTOLOGIN_DEV_DATA_DIR").map(PathBuf::from),
        memory_secrets: flag("AUTOLOGIN_DEV_MEMORY_SECRETS"),
        run_all_then_quit: flag("AUTOLOGIN_DEV_RUN_ALL"),
    }
}

/// Where to copy Chrome's debug log, if requested (debug builds only).
pub fn chrome_log_target() -> Option<PathBuf> {
    cfg!(debug_assertions).then(|| std::env::var_os("AUTOLOGIN_DEV_CHROME_LOG").map(PathBuf::from)).flatten()
}
