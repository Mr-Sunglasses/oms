//! Reading and setting the macOS desktop picture.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::{NSDictionary, NSString, NSURL};

/// Sets the wallpaper on every display (for the current Space).
pub fn set(path: &Path) -> Result<()> {
    let mtm = MainThreadMarker::new().context("wallpaper must be set from the main thread")?;
    let path = path
        .canonicalize()
        .with_context(|| format!("{} not found", path.display()))?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let workspace = NSWorkspace::sharedWorkspace();
    let options = NSDictionary::<NSString, AnyObject>::new();
    for screen in NSScreen::screens(mtm).iter() {
        unsafe { workspace.setDesktopImageURL_forScreen_options_error(&url, &screen, &options) }
            .map_err(|e| anyhow!("could not set wallpaper: {}", e.localizedDescription()))?;
    }
    Ok(())
}

/// The main display's current wallpaper.
pub fn current() -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    Some(PathBuf::from(url.path()?.to_string()))
}
