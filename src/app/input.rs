//! Keyboard and mouse handling.

use std::thread;
use std::time::Instant;

use anyhow::Result;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Position;

use super::{App, LIVE_DELAY, ModeFilter};
use crate::settings::Choice;
use crate::{actions, ghostty};

impl App {
    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
        } else if self.update_prompt.is_some() {
            self.update_prompt_key(key.code);
        } else if self.show_help {
            self.show_help = false;
        } else if self.filtering {
            self.filter_key(key);
        } else if !self.navigate(key.code) && !self.command(key.code) {
            let result = match key.code {
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
    }

    /// Moving through themes and wallpapers. Returns whether `code` was one.
    fn navigate(&mut self, code: KeyCode) -> bool {
        let all = self.themes.len() as isize;
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.move_theme(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_theme(1),
            KeyCode::PageUp => self.move_theme(-5),
            KeyCode::PageDown => self.move_theme(5),
            KeyCode::Home | KeyCode::Char('g') => self.move_theme(-all),
            KeyCode::End | KeyCode::Char('G') => self.move_theme(all),
            KeyCode::Left | KeyCode::Char('h') => self.move_wallpaper(-1),
            KeyCode::Right | KeyCode::Char('l') => self.move_wallpaper(1),
            _ => return false,
        }
        true
    }

    /// Keys that can't fail: applying, undo, views, quitting. Returns whether
    /// `code` was one.
    fn command(&mut self, code: KeyCode) -> bool {
        match code {
            KeyCode::Enter => self.start_apply(true, true),
            KeyCode::Char('t') => self.start_apply(true, false),
            KeyCode::Char('w') => self.start_apply(false, true),
            KeyCode::Char('u') => self.start_undo(),
            KeyCode::Char('r') => self.random(),
            KeyCode::Char('U') => self.update_now(),
            KeyCode::Char('?') => self.show_help = true,
            KeyCode::Char('/') => self.filtering = true,
            KeyCode::Tab => {
                self.mode_filter = match self.mode_filter {
                    ModeFilter::All => ModeFilter::Dark,
                    ModeFilter::Dark => ModeFilter::Light,
                    ModeFilter::Light => ModeFilter::All,
                };
                self.rebuild_order();
            }
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.rebuild_order();
            }
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            _ => return false,
        }
        true
    }

    /// `U`: update now, or check first if no new version is known yet.
    fn update_now(&mut self) {
        if self.update_available.is_some() {
            self.update_requested = true;
            self.quit = true;
        } else {
            self.update_wanted = true;
            self.status = "Checking for a new version…".into();
            let tx = self.updates_tx.clone();
            thread::spawn(move || super::tick::check_release(&tx));
        }
    }

    /// The "update now?" popup: y updates, n skips this version.
    fn update_prompt_key(&mut self, code: KeyCode) {
        let Some(version) = self.update_prompt.clone() else {
            return;
        };
        match code {
            KeyCode::Char('y' | 'Y') | KeyCode::Enter => {
                self.update_prompt = None;
                self.update_requested = true;
                self.quit = true;
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc => {
                self.update_prompt = None;
                self.settings.skipped_version = Some(version.clone());
                let _ = self.settings.save();
                self.status =
                    format!("OK. Update to {version} any time with U or `oms self-update`.");
            }
            _ => {}
        }
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent) {
        let at = Position::new(mouse.column, mouse.row);
        let in_wallpaper = self.wallpaper_area.contains(at);
        match mouse.kind {
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
            }
            _ => {}
        }
    }

    pub(super) fn filter_key(&mut self, key: KeyEvent) {
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
                self.move_theme(-1);
            }
            KeyCode::Down => {
                self.move_theme(1);
            }
            KeyCode::Char(c) => self.filter.push(c),
            _ => {}
        }
        self.rebuild_order();
    }

    pub(super) fn move_theme(&mut self, delta: isize) {
        if self.order.is_empty() {
            return;
        }
        let pos = self.position().unwrap_or(0) as isize;
        let last = self.order.len() as isize - 1;
        let target = self.order[(pos + delta).clamp(0, last) as usize].0;
        self.select(target);
    }

    pub(super) fn select(&mut self, index: usize) {
        if index != self.selected {
            self.selected = index;
            if self.live && !self.busy() {
                self.live_due = Some(Instant::now() + LIVE_DELAY);
            }
        }
        self.list.select(self.position());
    }

    pub(super) fn move_wallpaper(&mut self, delta: isize) {
        let count = self.theme().wallpapers.len() as isize;
        if count > 0 {
            let i = &mut self.wallpaper[self.selected];
            *i = (*i as isize + delta).rem_euclid(count) as usize;
        }
    }

    pub(super) fn random(&mut self) {
        let candidates: Vec<usize> = self
            .order
            .iter()
            .map(|(i, _)| *i)
            .filter(|&i| i != self.selected)
            .collect();
        if candidates.is_empty() {
            return;
        }
        let index = candidates[actions::random_below(candidates.len())];
        self.select(index);
        let count = self.theme().wallpapers.len();
        if count > 0 {
            self.wallpaper[self.selected] = actions::random_below(count);
        }
        self.status = format!("Picked {}. Press Enter to apply.", self.theme().name);
    }

    pub(super) fn toggle_favorite(&mut self) -> Result<()> {
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

    pub(super) fn set_mode(&mut self, dark: bool) -> Result<()> {
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

    pub(super) fn toggle_live(&mut self) -> Result<()> {
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
    pub(super) fn preview_selected(&mut self) -> Result<()> {
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
}
