//! Per-frame work: finished jobs, live preview, update checks and images.

use std::path::PathBuf;
use std::sync::mpsc::Sender;
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use ratatui_image::thread::ThreadProtocol;

use super::{App, Shown, UPDATE_EVERY_SECS};
use crate::repo::Repos;
use crate::settings::Settings;
use crate::{ghostty, update};

impl App {
    pub fn tick(&mut self) {
        self.ticks = self.ticks.wrapping_add(1);

        if let Some((_, rx)) = &self.job
            && let Ok(done) = rx.try_recv()
            && let Some((job, _)) = self.job.take()
        {
            self.finish_job(job, done);
        }

        if self.live_due.is_some_and(|due| Instant::now() >= due) {
            self.live_due = None;
            if let Err(e) = self.preview_selected() {
                self.status = format!("Error: {e:#}");
            }
        }

        self.poll_updates();
        self.loader.poll();
        self.request_images();
        self.update_preview();
        self.update_thumbs();
    }

    /// Handles results of the release check and the theme/wallpaper download.
    fn poll_updates(&mut self) {
        for message in self.updates.try_iter().collect::<Vec<_>>() {
            if let Some(version) = message.strip_prefix("release:") {
                let version = version.to_string();
                self.settings.latest_version = Some(version.clone());
                let _ = self.settings.save();
                self.update_available = Some(version.clone());
                if self.update_wanted {
                    self.update_requested = true;
                    self.quit = true;
                } else if self.settings.skipped_version.as_ref() != Some(&version) {
                    // Ask now, in this session.
                    self.update_prompt = Some(version);
                }
            } else if message == "current" {
                if self.update_wanted {
                    self.update_wanted = false;
                    self.status =
                        format!("✓ oms {} is the latest version.", env!("CARGO_PKG_VERSION"));
                }
                if self.settings.latest_version.is_some() {
                    self.settings.latest_version = None;
                    let _ = self.settings.save();
                    self.update_available = None;
                }
            } else if !self.busy() {
                self.status = message;
            }
        }
    }

    /// Asks the loader for the selected picture, its theme's others (for
    /// thumbnails), then the neighbouring themes' pictures.
    fn request_images(&mut self) {
        let theme = self.theme();
        let i = self.wallpaper[self.selected];
        let mut wanted: Vec<PathBuf> = Vec::new();
        if let Some(path) = theme.wallpapers.get(i) {
            wanted.push(path.clone());
        }
        wanted.extend(
            theme
                .wallpapers
                .iter()
                .filter(|p| Some(*p) != theme.wallpapers.get(i))
                .cloned(),
        );
        if let Some(pos) = self.position() {
            for p in [pos.wrapping_sub(1), pos + 1] {
                if let Some(&(t, _)) = self.order.get(p)
                    && let Some(path) = self.themes[t].wallpapers.get(self.wallpaper[t])
                {
                    wanted.push(path.clone());
                }
            }
        }
        // The loader works newest first, so the selected picture goes last.
        for path in wanted.iter().rev() {
            self.loader.request(path);
        }
    }

    /// Shows the selected wallpaper once it's loaded, keeping the previous
    /// picture up until the new one has been resized.
    fn update_preview(&mut self) {
        for response in self.encoded.try_iter().collect::<Vec<_>>() {
            if let (Ok(response), Some(shown)) = (response, self.image.as_mut())
                && shown.protocol.update_resized_protocol(response)
            {
                shown.ready = true;
                self.previous = None;
            }
        }

        let current = self.wallpaper_path().cloned();
        let shown = self.image.as_ref().map(|s| &s.path);
        if current.as_ref() != shown
            && let Some(path) = current
            && let Some(preview) = self.loader.get(&path)
        {
            let protocol = self.picker.new_resize_protocol(preview.image.clone());
            let size = (preview.image.width(), preview.image.height());
            let next = Shown {
                path,
                protocol: ThreadProtocol::new(self.encode_requests.clone(), Some(protocol)),
                size,
                ready: false,
            };
            if let Some(old) = self.image.replace(next)
                && old.ready
            {
                self.previous = Some(old);
            }
        }
    }

    /// Thumbnails for the selected theme, cropped to fill their boxes.
    fn update_thumbs(&mut self) {
        if self.thumbs_for != self.selected || self.thumbs_cropped_for != self.thumb_cells {
            self.thumbs.clear();
            self.thumbs_for = self.selected;
            self.thumbs_cropped_for = self.thumb_cells;
        }
        let (cols, rows) = self.thumb_cells;
        if cols == 0 || rows == 0 {
            return;
        }
        let font = self.picker.font_size();
        let target = (f64::from(cols) * f64::from(font.width))
            / (f64::from(rows) * f64::from(font.height.max(1)));
        for path in self.themes[self.selected].wallpapers.clone() {
            if !self.thumbs.contains_key(&path)
                && let Some(preview) = self.loader.get(&path)
            {
                let protocol = self
                    .picker
                    .new_resize_protocol(crop_to_aspect(&preview.thumb, target));
                self.thumbs.insert(path, protocol);
            }
        }
    }
}

/// In the background: look for a newer oms and pull new themes and wallpapers,
/// at most every few hours. Messages come back on the channel.
/// In the background: look for a newer oms (every launch; it's one small
/// request) and pull new themes and wallpapers (at most every few hours).
/// Results come back on the channel.
pub(super) fn start_update_check(repos: &Repos, settings: &mut Settings, tx: Sender<String>) {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let pull = now.saturating_sub(settings.last_update_check) >= UPDATE_EVERY_SECS;
    if pull {
        settings.last_update_check = now;
        let _ = settings.save();
    }
    let repos = repos.clone();
    thread::spawn(move || {
        check_release(&tx);
        if pull && let Ok(true) = repos.update() {
            let _ = ghostty::install_themes(&repos.themes);
            let _ = tx.send("Downloaded new themes and wallpapers. Reopen oms to see them.".into());
        }
    });
}

/// Sends "release:<version>" if a newer oms is out, else "current".
pub(super) fn check_release(tx: &Sender<String>) {
    let _ = tx.send(match update::available() {
        Some(version) => format!("release:{version}"),
        None => "current".into(),
    });
}

/// The largest centered part of `image` with the given width / height ratio.
pub(super) fn crop_to_aspect(image: &image::DynamicImage, aspect: f64) -> image::DynamicImage {
    let (w, h) = (f64::from(image.width()), f64::from(image.height()));
    if w / h > aspect {
        let new_w = (h * aspect).round().max(1.0);
        image.crop_imm(((w - new_w) / 2.0) as u32, 0, new_w as u32, h as u32)
    } else {
        let new_h = (w / aspect).round().max(1.0);
        image.crop_imm(0, ((h - new_h) / 2.0) as u32, w as u32, new_h as u32)
    }
}
