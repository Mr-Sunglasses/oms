//! Reading and setting the desktop wallpaper: `AppKit` and the wallpaper store on
//! macOS, the desktop's own tool on Linux (XFCE, GNOME, KDE, Hyprland, Sway).

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "linux")]
pub use linux::{current, set, set_current_space, set_everywhere};
#[cfg(target_os = "macos")]
pub use macos::{current, set, set_current_space, set_everywhere};

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

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
