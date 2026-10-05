//! TUI state and actions.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::images::Loader;
use crate::repo::{Repos, Theme};
use crate::settings::{Choice, Settings};
use crate::{actions, ghostty, update, wallpaper};

/// How long the selection has to rest before live preview recolors Ghostty.
const LIVE_DELAY: Duration = Duration::from_millis(120);
/// How often to look for new themes, wallpapers and oms releases.
const UPDATE_EVERY_SECS: u64 = 6 * 60 * 60;

/// Where a theme sits in the list.
#[derive(Clone, Copy, PartialEq)]
pub enum Group {
    Favorite,
    Recent,
    Other,
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
    /// A newer oms release, if the background check found one.
    pub update_available: Option<String>,
    updates: Option<Receiver<String>>,
    pub loader: Loader,
    picker: Picker,
    pub image: Option<(PathBuf, StatefulProtocol)>,
    /// Width / height of a terminal cell.
    pub cell_aspect: f64,
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
            show_help: false,
            saved_config: ghostty::read_config(&config),
            config,
            previewing: false,
            applied_theme,
            applied_wallpaper,
            live: true,
            live_due: None,
            status: "Press ? for all keys.".into(),
            update_available: None,
            updates,
            loader: Loader::new(),
            cell_aspect,
            picker,
            image: None,
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

    /// Favorites, then recently applied themes, then the rest, each matching
    /// the filter.
    fn rebuild_order(&mut self) {
        let matches: Vec<usize> = (0..self.themes.len())
            .filter(|&i| self.themes[i].contains(&self.filter))
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
            KeyCode::Up | KeyCode::Char('k') => self.move_theme(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_theme(1),
            KeyCode::PageUp => self.move_theme(-5),
            KeyCode::PageDown => self.move_theme(5),
            KeyCode::Home | KeyCode::Char('g') => self.move_theme(-(self.themes.len() as isize)),
            KeyCode::End | KeyCode::Char('G') => self.move_theme(self.themes.len() as isize),
            KeyCode::Left | KeyCode::Char('h') => self.move_wallpaper(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_wallpaper(1),
            KeyCode::Enter => self.apply_both(),
            KeyCode::Char('t') => self.apply_theme(),
            KeyCode::Char('w') => self.apply_wallpaper(),
            KeyCode::Char('r') => self.random(),
            KeyCode::Char('p') => self.toggle_live(),
            KeyCode::Char('f') => self.toggle_favorite(),
            KeyCode::Char('L') => self.set_mode(false),
            KeyCode::Char('D') => self.set_mode(true),
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
            if self.live {
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
        let choice = Choice {
            theme: self.theme().slug.clone(),
            wallpaper: self.wallpaper[self.selected],
        };
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

    fn apply_theme(&mut self) -> Result<()> {
        let outcome = actions::apply_theme(&self.themes[self.selected], &mut self.settings)?;
        self.saved_config = ghostty::read_config(&self.config);
        self.previewing = false;
        self.applied_theme = Some(self.theme().ghostty_name.clone());
        self.status = format!("{}.", outcome.message);
        self.rebuild_order();
        Ok(())
    }

    fn apply_wallpaper(&mut self) -> Result<()> {
        let index = self.wallpaper[self.selected];
        if self.theme().wallpapers.is_empty() {
            self.status = format!("{} has no wallpapers.", self.theme().name);
            return Ok(());
        }
        let path =
            actions::apply_wallpaper(&self.themes[self.selected], index, &mut self.settings)?;
        self.status = format!("Wallpaper set to {} on every Space.", file_name(&path));
        self.applied_wallpaper = Some(path);
        Ok(())
    }

    fn apply_both(&mut self) -> Result<()> {
        self.apply_theme()?;
        let theme_status = self.status.clone();
        self.apply_wallpaper()?;
        self.status = format!("{} Wallpaper set on every Space.", theme_status);
        Ok(())
    }

    /// Runs between frames: live preview, update notices, image loading.
    pub fn tick(&mut self) {
        if self.live_due.is_some_and(|due| Instant::now() >= due) {
            self.live_due = None;
            if let Err(e) = self.preview_selected() {
                self.status = format!("Error: {e:#}");
            }
        }

        if let Some(rx) = &self.updates {
            for message in rx.try_iter().collect::<Vec<_>>() {
                if let Some(version) = message.strip_prefix("release:") {
                    self.update_available = Some(version.to_string());
                } else {
                    self.status = message;
                }
            }
        }

        self.loader.poll();
        // The selected picture, then its neighbours and the next themes' pictures.
        let theme = self.theme();
        let count = theme.wallpapers.len();
        let i = self.wallpaper[self.selected];
        let mut wanted: Vec<PathBuf> = Vec::new();
        if count > 0 {
            wanted.push(theme.wallpapers[i].clone());
            wanted.push(theme.wallpapers[(i + 1) % count].clone());
            wanted.push(theme.wallpapers[(i + count - 1) % count].clone());
        }
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
            && let Some(image) = self.loader.get(&path)
        {
            self.image = Some((path, self.picker.new_resize_protocol(image.clone())));
        }
    }
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
