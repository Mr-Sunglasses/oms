//! TUI state and actions.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::ListState;
use ratatui_image::picker::Picker;
use ratatui_image::protocol::StatefulProtocol;

use crate::ghostty;
use crate::images::Loader;
use crate::repo::Theme;
use crate::wallpaper;

/// How long the selection has to rest before live preview recolors Ghostty.
const LIVE_DELAY: Duration = Duration::from_millis(120);

pub struct App {
    pub themes: Vec<Theme>,
    pub selected: usize,
    /// Selected wallpaper for each theme.
    pub wallpaper: Vec<usize>,
    pub list: ListState,
    config: PathBuf,
    /// The config as last applied, restored on quit after live previews.
    saved_config: Option<String>,
    previewing: bool,
    pub applied_theme: Option<String>,
    pub applied_wallpaper: Option<PathBuf>,
    pub live: bool,
    live_due: Option<Instant>,
    pub status: String,
    pub loader: Loader,
    picker: Picker,
    pub image: Option<(PathBuf, StatefulProtocol)>,
    /// Width / height of a terminal cell.
    pub cell_aspect: f64,
    pub quit: bool,
}

impl App {
    pub fn new(themes: Vec<Theme>, picker: Picker) -> Self {
        let config = ghostty::config_path();
        let applied_theme = ghostty::current_theme(&config);
        let applied_wallpaper = wallpaper::current();

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
        let wallpaper = themes
            .iter()
            .map(|t| {
                applied_wallpaper
                    .as_ref()
                    .and_then(|current| t.wallpapers.iter().position(|w| same_file(w, current)))
                    .unwrap_or(0)
            })
            .collect();

        let mut app = App {
            selected,
            wallpaper,
            list: ListState::default().with_selected(Some(selected)),
            saved_config: ghostty::read_config(&config),
            config,
            previewing: false,
            applied_theme,
            applied_wallpaper,
            live: true,
            live_due: None,
            status: String::new(),
            loader: Loader::new(),
            cell_aspect: {
                let font = picker.font_size();
                font.width as f64 / font.height.max(1) as f64
            },
            picker,
            image: None,
            quit: false,
            themes,
        };
        let home = dirs::home_dir().unwrap_or_default();
        let shown = app
            .config
            .strip_prefix(&home)
            .map(|p| format!("~/{}", p.display()));
        app.status = format!(
            "Ghostty config: {}",
            shown.unwrap_or_else(|_| app.config.display().to_string())
        );
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

    pub fn is_applied_wallpaper(&self, path: &PathBuf) -> bool {
        self.applied_wallpaper
            .as_ref()
            .is_some_and(|a| same_file(a, path))
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        let result = match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.quit = true;
                Ok(())
            }
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.quit = true;
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
            _ => Ok(()),
        };
        if let Err(e) = result {
            self.status = format!("Error: {e:#}");
        }
    }

    fn move_theme(&mut self, delta: isize) -> Result<()> {
        let last = self.themes.len() as isize - 1;
        self.select((self.selected as isize + delta).clamp(0, last) as usize);
        Ok(())
    }

    fn select(&mut self, index: usize) {
        if index != self.selected {
            self.selected = index;
            self.list.select(Some(index));
            if self.live {
                self.live_due = Some(Instant::now() + LIVE_DELAY);
            }
        }
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
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .subsec_nanos() as usize;
        let mut index = seed % self.themes.len();
        if index == self.selected && self.themes.len() > 1 {
            index = (index + 1) % self.themes.len();
        }
        self.select(index);
        let count = self.theme().wallpapers.len();
        if count > 0 {
            self.wallpaper[self.selected] = (seed / 7) % count;
        }
        self.status = format!("Picked {}. Press Enter to apply.", self.theme().name);
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
        let name = self.theme().ghostty_name.clone();
        ghostty::set_theme(&self.config, &name)?;
        self.saved_config = ghostty::read_config(&self.config);
        self.previewing = false;
        self.applied_theme = Some(name);
        self.status = format!("Applied the {} theme to Ghostty.", self.theme().name);
        Ok(())
    }

    fn apply_wallpaper(&mut self) -> Result<()> {
        let Some(path) = self.wallpaper_path().cloned() else {
            self.status = format!("{} has no wallpapers.", self.theme().name);
            return Ok(());
        };
        wallpaper::set(&path)?;
        self.status = format!("Wallpaper set to {}.", file_name(&path));
        self.applied_wallpaper = Some(path);
        Ok(())
    }

    fn apply_both(&mut self) -> Result<()> {
        self.apply_theme()?;
        self.apply_wallpaper()?;
        self.status = format!("Applied {} to Ghostty and the desktop.", self.theme().name);
        Ok(())
    }

    /// Runs between frames: live preview, image loading and prefetching.
    pub fn tick(&mut self) {
        if self.live_due.is_some_and(|due| Instant::now() >= due) {
            self.live_due = None;
            if let Err(e) = self.preview_selected() {
                self.status = format!("Error: {e:#}");
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
        for t in [self.selected.wrapping_sub(1), self.selected + 1] {
            if let Some(theme) = self.themes.get(t)
                && let Some(path) = theme.wallpapers.get(self.wallpaper[t])
            {
                wanted.push(path.clone());
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
