//! Linux: plays the live wallpaper with mpvpaper (Hyprland, Sway), restarting
//! it if it exits. mpvpaper pauses by itself while the wallpaper is covered.
//! An intro plays once: mpvpaper exits at its end, uncovering its still.
//! Omarchy plays videos itself, so the agent isn't involved there.

use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::log;
use crate::wallpaper::linux::wayland::{hyprland_has_wallpaper_tool, session_command};
use crate::wallpaper::linux::{Backend, detect};
use crate::wallpaper::live::{self, MPVPAPER, Player};

/// Muted, filling the screen (cropping what doesn't fit).
const MPV_OPTIONS: &str = "no-audio panscan=1.0 hwdec=auto";
/// An mpvpaper that exits sooner than this counts as failing to start.
const QUICK_EXIT: Duration = Duration::from_secs(10);
/// The longest wait before trying again after it keeps failing.
const MAX_BACKOFF: Duration = Duration::from_secs(300);

struct Running {
    video: PathBuf,
    child: Child,
    started: Instant,
}

pub struct Live {
    running: Option<Running>,
    /// Quick exits in a row, for backing off (e.g. at login, before the
    /// compositor is up).
    failures: u32,
    /// The video to start again, and when.
    retry: Option<(PathBuf, Instant)>,
}

impl Live {
    pub fn new(_woken: &Arc<AtomicBool>) -> Self {
        Self {
            running: None,
            failures: 0,
            retry: None,
        }
    }

    /// Whether a video is (or should be) playing, so the agent checks on it.
    pub fn active(&self) -> bool {
        self.running.is_some() || self.retry.is_some()
    }

    pub fn sync(&mut self, want: Option<&Path>) {
        if self.reap() {
            return;
        }
        let current = self.running.as_ref().map(|r| r.video.as_path());
        if current == want && current.is_some() {
            return;
        }
        if current.is_some() {
            self.stop();
        }
        let Some(video) = want else {
            self.failures = 0;
            self.retry = None;
            return;
        };
        match &self.retry {
            Some((v, at)) if v == video && Instant::now() < *at => return,
            Some((v, _)) if v != video => self.failures = 0,
            _ => {}
        }
        if live::player() != Player::Agent {
            log("not playing the live wallpaper: this desktop has no mpvpaper");
            self.retry = Some((video.to_path_buf(), Instant::now() + MAX_BACKOFF));
            return;
        }
        let spawned = session_command(MPVPAPER)
            .args(["-p", "-o", &mpv_options(video), "ALL"])
            .arg(video)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        match spawned {
            Ok(child) => {
                log(&format!("playing {}", video.display()));
                self.retry = None;
                self.running = Some(Running {
                    video: video.to_path_buf(),
                    child,
                    started: Instant::now(),
                });
            }
            Err(e) => {
                log(&format!("couldn't start {MPVPAPER}: {e}"));
                self.back_off(video);
            }
        }
    }

    /// Notices an mpvpaper that exited, and schedules a restart. Returns
    /// whether it was an intro that played to its end (and is now done).
    fn reap(&mut self) -> bool {
        let Some(running) = &mut self.running else {
            return false;
        };
        let Ok(Some(status)) = running.child.try_wait() else {
            return false;
        };
        let video = running.video.clone();
        let quick = running.started.elapsed() < QUICK_EXIT;
        self.running = None;
        if status.success() && live::is_intro(&video) {
            log("the intro finished");
            if let Err(e) = live::set_playing(None) {
                log(&format!("couldn't clear the intro: {e:#}"));
            }
            return true;
        }
        log(&format!("{MPVPAPER} exited ({status})"));
        if quick {
            self.back_off(&video);
        } else {
            self.failures = 0;
        }
        false
    }

    fn back_off(&mut self, video: &Path) {
        self.failures += 1;
        let at = Instant::now() + backoff(self.failures);
        self.retry = Some((video.to_path_buf(), at));
    }

    fn stop(&mut self) {
        if let Some(mut running) = self.running.take() {
            let _ = running.child.kill();
            let _ = running.child.wait();
            log("stopped the live wallpaper");
        }
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop();
    }
}

/// mpv options for `video`: looping, or for an intro played once. An intro
/// holds its last frame where nothing shows a picture under it (Hyprland
/// without hyprpaper or swww), as mpvpaper exiting would leave the screen bare.
fn mpv_options(video: &Path) -> String {
    if !live::is_intro(video) {
        return format!("{MPV_OPTIONS} loop-file=inf");
    }
    let bare = matches!(detect(), Ok(Backend::Hyprland)) && !hyprland_has_wallpaper_tool();
    if bare {
        format!("{MPV_OPTIONS} keep-open=yes")
    } else {
        MPV_OPTIONS.to_string()
    }
}

/// 5s, 10s, 20s, … up to `MAX_BACKOFF`.
fn backoff(failures: u32) -> Duration {
    Duration::from_secs(5)
        .saturating_mul(1 << failures.saturating_sub(1).min(16))
        .min(MAX_BACKOFF)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_off_up_to_a_limit() {
        assert_eq!(backoff(1), Duration::from_secs(5));
        assert_eq!(backoff(2), Duration::from_secs(10));
        assert_eq!(backoff(4), Duration::from_secs(40));
        assert_eq!(backoff(30), MAX_BACKOFF);
    }
}
