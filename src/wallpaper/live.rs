//! Live wallpapers: videos that play behind the desktop. This module decides
//! who plays one (the agent, the desktop itself, or nobody), remembers which
//! video is playing, and makes the still frame shown under it, in previews
//! and wherever a video can't play.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use image::ImageReader;
use image::codecs::jpeg::JpegEncoder;

use crate::settings::data_dir;

/// Video files oms can use as wallpapers. macOS plays what `AVFoundation` plays;
/// mpv and Omarchy also play `WebM` and Matroska.
#[cfg(target_os = "macos")]
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov"];
#[cfg(not(target_os = "macos"))]
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "m4v", "mov", "webm", "mkv"];

pub fn is_video(path: &Path) -> bool {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    VIDEO_EXTENSIONS.contains(&ext.as_str())
}

/// Who plays a video wallpaper on this desktop. (On macOS it's always the agent.)
#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub enum Player {
    /// The oms agent: a window behind the desktop icons on macOS, mpvpaper on
    /// Hyprland and Sway.
    Agent,
    /// The desktop plays videos itself (Omarchy), so oms just hands it over.
    Desktop,
    /// It could play here once this program is installed.
    Missing(&'static str),
    /// This desktop can't show a video as its wallpaper.
    Unsupported(&'static str),
}

#[cfg(target_os = "macos")]
pub fn player() -> Player {
    Player::Agent
}

#[cfg(target_os = "linux")]
pub fn player() -> Player {
    use super::linux::{Backend, detect, on_path};
    match detect() {
        Ok(Backend::Omarchy) => Player::Desktop,
        Ok(Backend::Hyprland | Backend::Sway) if on_path(MPVPAPER) => Player::Agent,
        Ok(Backend::Hyprland | Backend::Sway) => Player::Missing(MPVPAPER),
        Ok(backend) => Player::Unsupported(backend.name()),
        Err(_) => Player::Unsupported("this desktop"),
    }
}

/// Plays videos on wlroots compositors (Hyprland, Sway).
#[cfg(target_os = "linux")]
pub const MPVPAPER: &str = "mpvpaper";

/// Where the agent finds the video it should play.
fn state_path() -> PathBuf {
    data_dir().join("live-wallpaper")
}

/// The video the agent should be playing, if any (and it still exists).
pub fn playing() -> Option<PathBuf> {
    let text = fs::read_to_string(state_path()).ok()?;
    let path = PathBuf::from(text.trim_end_matches('\n'));
    path.is_file().then_some(path)
}

/// The state file's modification time, so the agent notices a change.
#[cfg(target_os = "linux")]
pub fn state_mtime() -> Option<std::time::SystemTime> {
    fs::metadata(state_path()).and_then(|m| m.modified()).ok()
}

/// Tells the agent to play `video`, or to stop (`None`), and starts or stops
/// the agent to match.
pub fn set_playing(video: Option<&Path>) -> Result<()> {
    let path = state_path();
    match video {
        Some(video) => {
            fs::create_dir_all(data_dir())?;
            let tmp = path.with_extension("tmp");
            fs::write(&tmp, format!("{}\n", video.display()))?;
            fs::rename(&tmp, &path)?;
        }
        None if !path.exists() => return Ok(()),
        None => fs::remove_file(&path)?,
    }
    crate::daemon::post_settings_changed();
    crate::daemon::sync(&crate::settings::Settings::load())
}

/// A still of the video's first frame (cached), to set as the ordinary
/// wallpaper under it and to preview it with.
pub fn poster(video: &Path) -> Result<PathBuf> {
    let target = poster_path(video);
    if target.exists() {
        return Ok(target);
    }
    let dir = target.parent().context("bad poster path")?;
    fs::create_dir_all(dir)?;
    let work = dir.join(format!(
        ".{}.work",
        target.file_stem().unwrap_or_default().to_string_lossy()
    ));
    let _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work)?;
    let result = extract_frame(video, &work).and_then(|frame| {
        // Re-encode as a JPEG: QuickLook writes PNGs, which are huge at 4K.
        let image = ImageReader::open(&frame)?.with_guessed_format()?.decode()?;
        let tmp = target.with_extension("tmp");
        let mut file = std::io::BufWriter::new(fs::File::create(&tmp)?);
        image
            .to_rgb8()
            .write_with_encoder(JpegEncoder::new_with_quality(&mut file, 92))?;
        drop(file);
        fs::rename(&tmp, &target)?;
        Ok(())
    });
    let _ = fs::remove_dir_all(&work);
    result.with_context(|| format!("couldn't get a frame of {}", video.display()))?;
    Ok(target)
}

/// `<data dir>/posters/<hash>.jpg`, keyed by the video's path, size and
/// modification time, so an edited video gets a new still.
pub fn poster_path(video: &Path) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    video.hash(&mut hasher);
    if let Ok(meta) = fs::metadata(video) {
        meta.len().hash(&mut hasher);
        if let Ok(modified) = meta.modified() {
            modified.hash(&mut hasher);
        }
    }
    data_dir()
        .join("posters")
        .join(format!("{:016x}.jpg", hasher.finish()))
}

/// Writes one frame of `video` into `dir` and returns its path. ffmpeg takes
/// exactly the first frame; macOS falls back to `QuickLook`, which is always there.
fn extract_frame(video: &Path, dir: &Path) -> Result<PathBuf> {
    let out = dir.join("frame.png");
    let ffmpeg = Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(video)
        .args(["-frames:v", "1"])
        .arg(&out)
        .stdin(Stdio::null())
        .output();
    if ffmpeg.is_ok_and(|o| o.status.success()) && out.exists() {
        return Ok(out);
    }
    #[cfg(target_os = "macos")]
    {
        // Writes `<dir>/<file name>.png`.
        let _ = Command::new("qlmanage")
            .args(["-t", "-s", "3840", "-o"])
            .arg(dir)
            .arg(video)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let name = video.file_name().unwrap_or_default().to_string_lossy();
        let thumb = dir.join(format!("{name}.png"));
        if thumb.exists() {
            return Ok(thumb);
        }
        bail!("QuickLook couldn't read it");
    }
    #[cfg(not(target_os = "macos"))]
    bail!("install ffmpeg so oms can show videos as stills")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn knows_videos_from_pictures() {
        assert!(is_video(Path::new("/w/ocean.mp4")));
        assert!(is_video(Path::new("/w/Ocean.MOV")));
        assert!(!is_video(Path::new("/w/ocean.jpg")));
        assert!(!is_video(Path::new("/w/ocean")));
        assert_eq!(
            is_video(Path::new("/w/ocean.webm")),
            cfg!(not(target_os = "macos"))
        );
    }

    #[test]
    fn posters_follow_the_file() {
        let a = poster_path(Path::new("/nowhere/a.mp4"));
        assert_eq!(a, poster_path(Path::new("/nowhere/a.mp4")));
        assert_ne!(a, poster_path(Path::new("/nowhere/b.mp4")));
        assert_eq!(a.extension().unwrap(), "jpg");
    }
}
