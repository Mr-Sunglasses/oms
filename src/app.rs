//! TUI state and actions.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::ListState;
use ratatui_image::errors::Errors;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;
use ratatui_image::thread::{ResizeRequest, ResizeResponse, ThreadProtocol};

use crate::images::Loader;
use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings};
use crate::{actions, apps, ghostty, update, wallpaper};

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
    updates: Option<Receiver<String>>,
    job: Option<(Job, Receiver<JobDone>)>,
    undo: Vec<Snapshot>,
    ticks: usize,
    pub loader: Loader,
    picker: Picker,
    /// The big preview, resized and encoded on a worker thread.
    pub image: Option<(PathBuf, ThreadProtocol)>,
    encode_requests: Sender<ResizeRequest>,
    encoded: Receiver<Result<ResizeResponse, Errors>>,
    /// Thumbnails of the selected theme's wallpapers.
    pub thumbs: HashMap<PathBuf, StatefulProtocol>,
    thumbs_for: usize,
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
        let updates = start_update_check(repos, &mut settings);
        let cell_aspect = {
            let font = picker.font_size();
            font.width as f64 / font.height.max(1) as f64
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
            job: None,
            undo: Vec::new(),
            ticks: 0,
            loader: Loader::new(prefetch),
            cell_aspect,
            picker,
            image: None,
            encode_requests,
            encoded,
            thumbs: HashMap::new(),
            thumbs_for: usize::MAX,
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

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
            return;
        }
        if self.show_help {
            self.show_help = false;
            return;
        }
        if self.filtering {
            self.filter_key(key);
            return;
        }
        let result = match key.code {
            KeyCode::Char('q') => {
                self.quit = true;
                Ok(())
            }
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.rebuild_order();
                Ok(())
            }
            KeyCode::Esc => {
                self.quit = true;
                Ok(())
            }
            KeyCode::Char('?') => {
                self.show_help = true;
                Ok(())
            }
            KeyCode::Char('/') => {
                self.filtering = true;
                Ok(())
            }
            KeyCode::Tab => {
                self.mode_filter = match self.mode_filter {
                    ModeFilter::All => ModeFilter::Dark,
                    ModeFilter::Dark => ModeFilter::Light,
                    ModeFilter::Light => ModeFilter::All,
                };
                self.rebuild_order();
                Ok(())
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_theme(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_theme(1),
            KeyCode::PageUp => self.move_theme(-5),
            KeyCode::PageDown => self.move_theme(5),
            KeyCode::Home | KeyCode::Char('g') => self.move_theme(-(self.themes.len() as isize)),
            KeyCode::End | KeyCode::Char('G') => self.move_theme(self.themes.len() as isize),
            KeyCode::Left | KeyCode::Char('h') => self.move_wallpaper(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_wallpaper(1),
            KeyCode::Enter => self.start_apply(true, true),
            KeyCode::Char('t') => self.start_apply(true, false),
            KeyCode::Char('w') => self.start_apply(false, true),
            KeyCode::Char('u') => self.start_undo(),
            KeyCode::Char('r') => self.random(),
            KeyCode::Char('p') => self.toggle_live(),
            KeyCode::Char('f') => self.toggle_favorite(),
            KeyCode::Char('L') => self.set_mode(false),
            KeyCode::Char('D') => self.set_mode(true),
            KeyCode::Char('U') => {
                if self.update_available.is_some() {
                    self.update_requested = true;
                    self.quit = true;
                } else {
                    self.status = "oms is up to date.".into();
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.status = format!("Error: {e:#}");
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let at = Position::new(mouse.column, mouse.row);
        let in_wallpaper = self.wallpaper_area.contains(at);
        let result = match mouse.kind {
            MouseEventKind::ScrollDown if in_wallpaper => self.move_wallpaper(1),
            MouseEventKind::ScrollUp if in_wallpaper => self.move_wallpaper(-1),
            MouseEventKind::ScrollDown => self.move_theme(1),
            MouseEventKind::ScrollUp => self.move_theme(-1),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.list_inner.contains(at) {
                    let row = (at.y - self.list_inner.y) as usize + self.list.offset();
                    if let Some(&(i, _)) = self.order.get(row) {
                        self.select(i);
                    }
                } else if let Some(&(_, index)) =
                    self.thumb_rects.iter().find(|(r, _)| r.contains(at))
                {
                    self.wallpaper[self.selected] = index;
                }
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.status = format!("Error: {e:#}");
        }
    }

    fn filter_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.filtering = false;
            }
            KeyCode::Enter => self.filtering = false,
            KeyCode::Backspace => {
                self.filter.pop();
            }
            KeyCode::Up => {
                let _ = self.move_theme(-1);
            }
            KeyCode::Down => {
                let _ = self.move_theme(1);
            }
            KeyCode::Char(c) => self.filter.push(c),
            _ => {}
        }
        self.rebuild_order();
    }

    fn move_theme(&mut self, delta: isize) -> Result<()> {
        if self.order.is_empty() {
            return Ok(());
        }
        let pos = self.position().unwrap_or(0) as isize;
        let last = self.order.len() as isize - 1;
        let target = self.order[(pos + delta).clamp(0, last) as usize].0;
        self.select(target);
        Ok(())
    }

    fn select(&mut self, index: usize) {
        if index != self.selected {
            self.selected = index;
            if self.live && !self.busy() {
                self.live_due = Some(Instant::now() + LIVE_DELAY);
            }
        }
        self.list.select(self.position());
    }

    fn move_wallpaper(&mut self, delta: isize) -> Result<()> {
        let count = self.theme().wallpapers.len() as isize;
        if count > 0 {
            let i = &mut self.wallpaper[self.selected];
            *i = (*i as isize + delta).rem_euclid(count) as usize;
        }
        Ok(())
    }

    fn random(&mut self) -> Result<()> {
        let candidates: Vec<usize> = self
            .order
            .iter()
            .map(|(i, _)| *i)
            .filter(|&i| i != self.selected)
            .collect();
        if candidates.is_empty() {
            return Ok(());
        }
        let index = candidates[actions::random_below(candidates.len())];
        self.select(index);
        let count = self.theme().wallpapers.len();
        if count > 0 {
            self.wallpaper[self.selected] = actions::random_below(count);
        }
        self.status = format!("Picked {}. Press Enter to apply.", self.theme().name);
        Ok(())
    }

    fn toggle_favorite(&mut self) -> Result<()> {
        let slug = self.theme().slug.clone();
        let added = self.settings.toggle_favorite(&slug);
        self.settings.save()?;
        self.status = if added {
            format!("★ {} is a favorite.", self.theme().name)
        } else {
            format!("{} is no longer a favorite.", self.theme().name)
        };
        self.rebuild_order();
        Ok(())
    }

    fn set_mode(&mut self, dark: bool) -> Result<()> {
        if self.busy() {
            return Ok(());
        }
        let choice = Choice {
            theme: self.theme().slug.clone(),
            wallpaper: self.wallpaper[self.selected],
        };
        self.push_undo();
        let outcome = actions::set_mode_theme(&self.themes, dark, choice, &mut self.settings)?;
        self.saved_config = ghostty::read_config(&self.config);
        self.applied_theme = ghostty::current_theme(&self.config);
        self.previewing = false;
        self.status = outcome.message;
        Ok(())
    }

    fn toggle_live(&mut self) -> Result<()> {
        self.live = !self.live;
        if self.live {
            self.preview_selected()?;
            self.status = "Live preview on: Ghostty follows the selected theme.".into();
        } else {
            self.restore()?;
            self.status = "Live preview off.".into();
        }
        Ok(())
    }

    /// Recolors Ghostty with the selected theme without committing it.
    fn preview_selected(&mut self) -> Result<()> {
        let name = self.theme().ghostty_name.clone();
        if self.applied_theme.as_deref() == Some(name.as_str()) {
            return self.restore();
        }
        let text = ghostty::with_theme(self.saved_config.as_deref().unwrap_or_default(), &name);
        ghostty::write_config(&self.config, Some(&text))?;
        self.previewing = true;
        Ok(())
    }

    /// Puts back the last applied config if a live preview changed it.
    pub fn restore(&mut self) -> Result<()> {
        if self.previewing {
            ghostty::write_config(&self.config, self.saved_config.as_deref())?;
            self.previewing = false;
        }
        Ok(())
    }

    fn applied_theme_obj(&self) -> Option<Theme> {
        let current = self.applied_theme.as_deref()?;
        self.themes
            .iter()
            .find(|t| t.ghostty_name == current)
            .cloned()
    }

    fn push_undo(&mut self) {
        self.undo.push(Snapshot {
            config: self.saved_config.clone(),
            wallpaper: self.applied_wallpaper.clone(),
            theme: self.applied_theme_obj(),
        });
    }

    /// Applies the selected theme and/or wallpaper on a worker thread.
    fn start_apply(&mut self, theme: bool, wallpaper: bool) -> Result<()> {
        if self.busy() {
            return Ok(());
        }
        let t = self.themes[self.selected].clone();
        let index = self.wallpaper[self.selected];
        let wallpaper = if wallpaper {
            match t.wallpapers.get(index) {
                Some(path) => Some((index, path.clone())),
                None if !theme => {
                    self.status = format!("{} has no wallpapers.", t.name);
                    return Ok(());
                }
                None => None,
            }
        } else {
            None
        };
        self.live_due = None;
        self.push_undo();
        let apps = self.settings.apps.clone();
        let (tx, rx) = channel();
        let job_theme = theme.then(|| t.clone());
        let job_wallpaper = wallpaper.as_ref().map(|(_, p)| p.clone());
        thread::spawn(move || {
            let _ = tx.send(run_job(
                job_theme.as_ref(),
                &apps,
                job_wallpaper.as_deref(),
                None,
            ));
        });
        self.status = match (theme, &wallpaper) {
            (true, Some(_)) => format!("Applying {} and its wallpaper…", t.name),
            (true, None) => format!("Applying {}…", t.name),
            _ => "Setting the wallpaper…".into(),
        };
        self.job = Some((
            Job::Apply {
                theme: theme.then_some(self.selected),
                wallpaper,
            },
            rx,
        ));
        Ok(())
    }

    /// Puts back what was applied before the last change.
    fn start_undo(&mut self) -> Result<()> {
        if self.busy() {
            return Ok(());
        }
        let Some(snapshot) = self.undo.pop() else {
            self.status = "Nothing to undo.".into();
            return Ok(());
        };
        self.live_due = None;
        let apps = self.settings.apps.clone();
        let (tx, rx) = channel();
        let s = snapshot.clone();
        thread::spawn(move || {
            let _ = tx.send(run_job(
                s.theme.as_ref(),
                &apps,
                s.wallpaper.as_deref(),
                Some(s.config.as_deref()),
            ));
        });
        self.status = "Undoing…".into();
        self.job = Some((Job::Undo(snapshot), rx));
        Ok(())
    }

    fn finish_job(&mut self, job: Job, done: JobDone) {
        if let Some(path) = &done.wallpaper_fallback
            && let Err(e) = wallpaper::set_current_space(path)
        {
            self.status = format!("Error: {e:#}");
            return;
        }
        if let Some(e) = done.error {
            self.status = format!("Error: {e}");
            self.undo.pop();
            return;
        }
        self.saved_config = ghostty::read_config(&self.config);
        self.applied_theme = ghostty::current_theme(&self.config);
        self.previewing = false;
        match job {
            Job::Apply { theme, wallpaper } => {
                let mut parts = Vec::new();
                if let Some(i) = theme {
                    let note = actions::record_theme(&self.themes[i], &mut self.settings)
                        .unwrap_or_default();
                    parts.push(format!(
                        "Applied {}{}{note}",
                        self.themes[i].name, done.apps
                    ));
                }
                if let Some((index, path)) = wallpaper {
                    let slug = self.theme().slug.clone();
                    self.settings.wallpaper_index.insert(slug, index);
                    let _ = self.settings.save();
                    parts.push(format!(
                        "Wallpaper set to {} on every Space",
                        file_name(&path)
                    ));
                    self.applied_wallpaper = Some(path);
                }
                self.status = format!("✓ {}.", parts.join(". "));
                self.rebuild_order();
            }
            Job::Undo(snapshot) => {
                self.applied_wallpaper = snapshot.wallpaper;
                let name = snapshot
                    .theme
                    .map(|t| t.name)
                    .unwrap_or_else(|| "your previous theme".into());
                self.status = format!(
                    "↶ Back to {name}{}. ({} more to undo)",
                    done.apps,
                    self.undo.len()
                );
            }
        }
    }

    /// Runs between frames: jobs, live preview, update notices, images.
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

        if let Some(rx) = &self.updates {
            for message in rx.try_iter().collect::<Vec<_>>() {
                if let Some(version) = message.strip_prefix("release:") {
                    self.settings.latest_version = Some(version.to_string());
                    let _ = self.settings.save();
                    self.update_available = Some(version.to_string());
                } else if !self.busy() {
                    self.status = message;
                }
            }
        }

        self.loader.poll();
        for response in self.encoded.try_iter().collect::<Vec<_>>() {
            if let (Ok(response), Some((_, protocol))) = (response, self.image.as_mut()) {
                protocol.update_resized_protocol(response);
            }
        }

        // The selected picture, its theme's others (for thumbnails), then the
        // neighbouring themes' pictures.
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

        let current = self.wallpaper_path().cloned();
        let shown = self.image.as_ref().map(|(p, _)| p);
        if current.as_ref() != shown
            && let Some(path) = current
            && let Some(preview) = self.loader.get(&path)
        {
            let protocol = self.picker.new_resize_protocol(preview.image.clone());
            self.image = Some((
                path,
                ThreadProtocol::new(self.encode_requests.clone(), Some(protocol)),
            ));
        }

        // Thumbnails for the selected theme.
        if self.thumbs_for != self.selected {
            self.thumbs.clear();
            self.thumbs_for = self.selected;
        }
        for path in self.themes[self.selected].wallpapers.clone() {
            if !self.thumbs.contains_key(&path)
                && let Some(preview) = self.loader.get(&path)
            {
                let protocol = self.picker.new_resize_protocol(preview.thumb.clone());
                self.thumbs.insert(path, protocol);
            }
        }
    }
}

/// Applies a theme's files and apps, the wallpaper, or (for undo) a saved
/// config. Runs on a worker thread.
fn run_job(
    theme: Option<&Theme>,
    apps: &[String],
    wallpaper: Option<&Path>,
    config: Option<Option<&str>>,
) -> JobDone {
    let mut done = JobDone {
        apps: String::new(),
        wallpaper_fallback: None,
        error: None,
    };
    let result = (|| -> Result<()> {
        match (config, theme) {
            // Undo: put the old config back, and the old theme's app colors.
            (Some(config), theme) => {
                ghostty::write_config(&ghostty::config_path(), config)?;
                if let Some(theme) = theme {
                    let (ok, _) = apps::apply(theme, apps);
                    if !ok.is_empty() {
                        done.apps = format!(" (+ {})", ok.join(", "));
                    }
                }
            }
            (None, Some(theme)) => done.apps = actions::theme_effects(theme, apps)?,
            (None, None) => {}
        }
        if let Some(path) = wallpaper
            && wallpaper::set_everywhere(path).is_err()
        {
            done.wallpaper_fallback = Some(path.to_path_buf());
        }
        Ok(())
    })();
    if let Err(e) = result {
        done.error = Some(format!("{e:#}"));
    }
    done
}

/// In the background: look for a newer oms and pull new themes and wallpapers,
/// at most every few hours. Messages come back on the channel.
fn start_update_check(repos: &Repos, settings: &mut Settings) -> Option<Receiver<String>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    if now.saturating_sub(settings.last_update_check) < UPDATE_EVERY_SECS {
        return None;
    }
    settings.last_update_check = now;
    let _ = settings.save();
    let (tx, rx) = channel();
    let repos = repos.clone();
    thread::spawn(move || {
        if let Some(version) = update::available() {
            let _ = tx.send(format!("release:{version}"));
        }
        if let Ok(true) = repos.update() {
            let _ = ghostty::install_themes(&repos.themes);
            let _ = tx.send("Downloaded new themes and wallpapers. Reopen oms to see them.".into());
        }
    });
    Some(rx)
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
