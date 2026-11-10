//! Reading and setting the desktop wallpaper: `AppKit` and the wallpaper store on
//! macOS, the desktop's own tool on Linux (Omarchy, XFCE, GNOME, KDE,
//! Hyprland, Sway). Videos are live wallpapers; see `live`.

#[cfg(target_os = "linux")]
pub mod linux;
pub mod live;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::Result;

use live::Player;

/// Sets the wallpaper on every display (and Space or workspace). Downloads it
/// first if only its preview is here. A video plays live where it can.
pub fn set(path: &Path) -> Result<()> {
    apply(path, platform::set)
}

/// Like `set`, but safe to call off the main thread. On macOS it can fail
/// where `set_current_space` (on the main thread) still works.
pub fn set_everywhere(path: &Path) -> Result<()> {
    apply(path, platform::set_everywhere)
}

/// The fallback for `set_everywhere` on macOS; the same as `set` on Linux.
pub fn set_current_space(path: &Path) -> Result<()> {
    apply(path, platform::set_current_space)
}

/// Sets a picture with `set_still`, or for a video: hands it to a desktop
/// that plays videos itself, or sets its first frame and has the agent play
/// it on top. Where nothing can play it, the first frame stays.
fn apply(path: &Path, set_still: fn(&Path) -> Result<()>) -> Result<()> {
    if !live::is_video(path) {
        set_still(path)?;
        return live::set_playing(None);
    }
    crate::repo::ensure_local(path)?;
    let path = &path.canonicalize()?;
    match live::player() {
        Player::Desktop => {
            set_still(path)?;
            live::set_playing(None)
        }
        Player::Agent => {
            // The still is what shows on the lock screen and while the agent
            // starts. On Hyprland without hyprpaper there's nowhere to set it,
            // and the video plays all the same.
            let still = live::poster(path).and_then(|poster| set_still(&poster));
            if cfg!(target_os = "macos") {
                still?;
            }
            live::set_playing(Some(path))
        }
        Player::Missing(_) | Player::Unsupported(_) => {
            set_still(&live::poster(path)?)?;
            live::set_playing(None)
        }
    }
}

/// The current wallpaper, if the desktop says: the video while one plays.
pub fn current() -> Option<PathBuf> {
    live::playing().or_else(platform::current)
}

/// What the desktop itself shows under a live wallpaper (its still frame,
/// unless something else changed it since).
pub fn current_still() -> Option<PathBuf> {
    platform::current()
}

/// Whether `program` is installed (in a `PATH` folder).
pub fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(program).is_file()))
}

/// A `file://` URL for `path`, percent-encoded.
pub fn file_url(path: &Path) -> String {
    let mut out = String::from("file://");
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// The path in a `file://` URL (the reverse of `file_url`); `None` for other URLs.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn url_path(url: &str) -> Option<PathBuf> {
    let encoded = url.strip_prefix("file://")?.as_bytes();
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut i = 0;
    while i < encoded.len() {
        let hex = encoded
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (encoded[i], hex) {
            (b'%', Some(byte)) => {
                bytes.push(byte);
                i += 3;
            }
            (byte, _) => {
                bytes.push(byte);
                i += 1;
            }
        }
    }
    Some(PathBuf::from(String::from_utf8_lossy(&bytes).into_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_urls_round_trip() {
        let path = Path::new("/home/me/Wallpapers/1 - Tokyo Night (2).png");
        let url = file_url(path);
        assert_eq!(
            url,
            "file:///home/me/Wallpapers/1%20-%20Tokyo%20Night%20%282%29.png"
        );
        assert_eq!(url_path(&url).as_deref(), Some(path));
        assert_eq!(url_path("https://example.com/a.png"), None);
    }
}
