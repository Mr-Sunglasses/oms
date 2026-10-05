//! TUI state: the themes, what's selected and applied, and what's in flight.
//!
//! Keys and mouse are in `input`, background apply/undo in `jobs`, and the
//! per-frame work (images, update checks) in `tick`.

mod input;
mod jobs;
mod tick;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::layout::Rect;
use ratatui::widgets::ListState;
use ratatui_image::errors::Errors;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::thread::{ResizeRequest, ResizeResponse, ThreadProtocol};

use crate::images::Loader;
use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings};
use crate::{ghostty, wallpaper};

/// How long the selection has to rest before live preview recolors Ghostty.
const LIVE_DELAY: Duration = Duration::from_millis(120);
/// How often to look for new themes, wallpapers and oms releases.
const UPDATE_EVERY_SECS: u64 = 6 * 60 * 60;
const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Where a theme sits in the list.
#[derive(Clone, Copy, PartialEq)]
pub enum Group {
    Favorite,
    Recent,
    Other,
}

/// Which themes the list shows (Tab cycles).
#[derive(Clone, Copy, PartialEq)]
pub enum ModeFilter {
    All,
    Dark,
    Light,
}

impl ModeFilter {
    pub fn label(self) -> &'static str {
        match self {
            ModeFilter::All => "all",
            ModeFilter::Dark => "dark",
            ModeFilter::Light => "light",
        }
    }
}

/// A wallpaper preview on screen.
pub struct Shown {
    pub path: PathBuf,
    pub protocol: ThreadProtocol,
    /// Pixel size of the picture, for fitting it to the pane.
    pub size: (u32, u32),
    /// Whether it has been resized and encoded, so it can be drawn.
    pub ready: bool,
}

/// What was applied before, so `u` can put it back.
#[derive(Clone)]
struct Snapshot {
    config: Option<String>,
    wallpaper: Option<PathBuf>,
    theme: Option<Theme>,
}

/// Work done off the UI thread, so the picker never freezes.
enum Job {
    Apply {
        theme: Option<usize>,
        wallpaper: Option<(usize, PathBuf)>,
    },
    Undo(Snapshot),
}

struct JobDone {
    apps: String,
    /// The wallpaper store couldn't be used; set it on the main thread instead.
    wallpaper_fallback: Option<PathBuf>,
    error: Option<String>,
}

// The flags are independent switches (filtering, help, live preview…), not a
// state machine, so separate bools are the clearest form.
#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub themes: Vec<Theme>,
    pub settings: Settings,
    /// Index into `themes` of the selected theme.
    pub selected: usize,
    /// The visible themes, in list order, with their group.
    pub order: Vec<(usize, Group)>,
    /// Selected wallpaper for each theme.
    pub wallpaper: Vec<usize>,
    pub list: ListState,
    pub filter: String,
    pub filtering: bool,
    pub mode_filter: ModeFilter,
    pub show_help: bool,
    config: PathBuf,
    /// The config as last applied, restored on quit after live previews.
    saved_config: Option<String>,
    previewing: bool,
    pub applied_theme: Option<String>,
    pub applied_wallpaper: Option<PathBuf>,
    pub live: bool,
    live_due: Option<Instant>,
    pub status: String,
    /// A newer oms release, if one is known.
    pub update_available: Option<String>,
    /// Set by `U`: quit and update oms.
    pub update_requested: bool,
    updates: Receiver<String>,
    updates_tx: Sender<String>,
    /// Asking "update now?" in a popup.
    pub update_prompt: Option<String>,
    /// `U` was pressed before the answer was known.
    update_wanted: bool,
    job: Option<(Job, Receiver<JobDone>)>,
    undo: Vec<Snapshot>,
    ticks: usize,
    pub loader: Loader,
    picker: Picker,
    /// The big preview, resized and encoded on a worker thread.
    pub image: Option<Shown>,
    /// The picture shown before, kept on screen until `image` is ready, so
    /// switching wallpapers never flashes blank.
    pub previous: Option<Shown>,
    encode_requests: Sender<ResizeRequest>,
    encoded: Receiver<Result<ResizeResponse, Errors>>,
    /// Thumbnails of the selected theme's wallpapers.
    pub thumbs: HashMap<PathBuf, StatefulProtocol>,
    thumbs_for: usize,
    /// Size in cells of a thumbnail box's inside, set by the UI; thumbnails
    /// are cropped to exactly this shape so they fill the box.
    pub thumb_cells: (u16, u16),
    thumbs_cropped_for: (u16, u16),
    /// Width / height of a terminal cell.
    pub cell_aspect: f64,
    /// Filled in by the UI each frame, for mouse clicks.
    pub list_inner: Rect,
    pub wallpaper_area: Rect,
    pub thumb_rects: Vec<(Rect, usize)>,
    pub quit: bool,
}

impl App {
    pub fn new(themes: Vec<Theme>, mut settings: Settings, picker: Picker, repos: &Repos) -> Self {
        let config = ghostty::config_path();
        let applied_theme = ghostty::current_theme(&config);
        let applied_wallpaper = wallpaper::current();

        let wallpaper = themes
            .iter()
            .map(|t| {
                applied_wallpaper
                    .as_ref()
                    .and_then(|current| t.wallpapers.iter().position(|w| same_file(w, current)))
                    .or_else(|| settings.wallpaper_index.get(&t.slug).copied())
                    .filter(|&i| i < t.wallpapers.len())
                    .unwrap_or(0)
            })
            .collect();
        let selected = applied_theme
            .as_deref()
            .and_then(|t| themes.iter().position(|th| th.ghostty_name == t))
            .or_else(|| {
                let current = applied_wallpaper.as_ref()?;
                themes
                    .iter()
                    .position(|t| t.wallpapers.iter().any(|w| same_file(w, current)))
            })
            .unwrap_or(0);

        // Cache previews of every wallpaper in the background, the selected theme's first.
        let mut prefetch: Vec<PathBuf> = themes[selected].wallpapers.clone();
        for t in &themes {
            prefetch.extend(t.wallpapers.iter().cloned());
        }

        let (encode_requests, encode_inbox) = channel::<ResizeRequest>();
        let (encoded_tx, encoded) = channel();
        thread::spawn(move || {
            while let Ok(request) = encode_inbox.recv() {
                if encoded_tx.send(request.resize_encode()).is_err() {
                    return;
                }
            }
        });

        let update_available = settings.newer_release();
        let (updates_tx, updates) = channel::<String>();
        tick::start_update_check(repos, &mut settings, updates_tx.clone());
        let cell_aspect = {
            let font = picker.font_size();
            f64::from(font.width) / f64::from(font.height.max(1))
        };
        let mut app = App {
            themes,
            settings,
            selected,
            order: Vec::new(),
            wallpaper,
            list: ListState::default(),
            filter: String::new(),
            filtering: false,
            mode_filter: ModeFilter::All,
            show_help: false,
            saved_config: ghostty::read_config(&config),
            config,
            previewing: false,
            applied_theme,
            applied_wallpaper,
            live: true,
            live_due: None,
            status: "Press ? for all keys.".into(),
            update_available,
            update_requested: false,
            updates,
            updates_tx,
            update_prompt: None,
            update_wanted: false,
            job: None,
            undo: Vec::new(),
            ticks: 0,
            loader: Loader::new(prefetch),
            cell_aspect,
            picker,
            image: None,
            previous: None,
            encode_requests,
            encoded,
            thumbs: HashMap::new(),
            thumbs_for: usize::MAX,
            thumb_cells: (0, 0),
            thumbs_cropped_for: (0, 0),
            list_inner: Rect::default(),
            wallpaper_area: Rect::default(),
            thumb_rects: Vec::new(),
            quit: false,
        };
        app.rebuild_order();
        app
    }

    pub fn theme(&self) -> &Theme {
        &self.themes[self.selected]
    }

    pub fn wallpaper_path(&self) -> Option<&PathBuf> {
        self.theme().wallpapers.get(self.wallpaper[self.selected])
    }

    pub fn is_applied_theme(&self, theme: &Theme) -> bool {
        self.applied_theme.as_deref() == Some(theme.ghostty_name.as_str())
    }

    /// "☀" / "☾" when the theme is the light or dark pick.
    pub fn mode_marks(&self, theme: &Theme) -> &'static str {
        let is = |c: &Option<Choice>| c.as_ref().is_some_and(|c| c.theme == theme.slug);
        match (is(&self.settings.light), is(&self.settings.dark)) {
            (true, true) => "☀☾",
            (true, false) => "☀",
            (false, true) => "☾",
            _ => "",
        }
    }

    pub fn is_applied_wallpaper(&self, path: &PathBuf) -> bool {
        self.applied_wallpaper
            .as_ref()
            .is_some_and(|a| same_file(a, path))
    }

    /// The status line, with a spinner while something is being applied.
    pub fn status_line(&self) -> String {
        if self.job.is_some() {
            format!(
                "{} {}",
                SPINNER[self.ticks / 2 % SPINNER.len()],
                self.status
            )
        } else {
            self.status.clone()
        }
    }

    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    /// Favorites, then recently applied themes, then the rest, each matching
    /// the filter and the light/dark choice.
    fn rebuild_order(&mut self) {
        let mode = self.mode_filter;
        let matches: Vec<usize> = (0..self.themes.len())
            .filter(|&i| {
                let t = &self.themes[i];
                t.contains(&self.filter)
                    && match mode {
                        ModeFilter::All => true,
                        ModeFilter::Dark => !t.is_light(),
                        ModeFilter::Light => t.is_light(),
                    }
            })
            .collect();
        let slug = |i: usize| self.themes[i].slug.as_str();
        let mut order: Vec<(usize, Group)> = Vec::new();
        for &i in &matches {
            if self.settings.is_favorite(slug(i)) {
                order.push((i, Group::Favorite));
            }
        }
        for recent in &self.settings.recent {
            if let Some(&i) = matches.iter().find(|&&i| slug(i) == recent)
                && !order.iter().any(|(o, _)| *o == i)
            {
                order.push((i, Group::Recent));
            }
        }
        for &i in &matches {
            if !order.iter().any(|(o, _)| *o == i) {
                order.push((i, Group::Other));
            }
        }
        self.order = order;
        if !self.order.iter().any(|(i, _)| *i == self.selected)
            && let Some(&(first, _)) = self.order.first()
        {
            self.select(first);
        }
        self.list.select(self.position());
    }

    fn position(&self) -> Option<usize> {
        self.order.iter().position(|(i, _)| *i == self.selected)
    }
}

fn same_file(a: &PathBuf, b: &PathBuf) -> bool {
    a == b
        || a.canonicalize()
            .ok()
            .is_some_and(|a| b.canonicalize().ok() == Some(a))
}

pub fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}
