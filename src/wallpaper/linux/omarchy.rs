//! Omarchy: its shell draws background pictures and OWE plays videos, so oms
//! hands both to `omarchy-theme-bg-set` (no hyprpaper or swaybg there).

use std::path::{Path, PathBuf};

use anyhow::Result;

use super::run;
use super::wayland::session_command;

pub const SET_COMMAND: &str = "omarchy-theme-bg-set";

pub fn set(path: &Path) -> Result<()> {
    run(session_command(SET_COMMAND).arg(path))?;
    Ok(())
}

/// Omarchy points this link at the background it shows.
fn current_link() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".local/state/omarchy/current/background"))
}

pub fn current() -> Option<PathBuf> {
    std::fs::read_link(current_link()?).ok()
}
