//! Applying and undoing on a worker thread, so the picker never freezes.

use std::path::Path;
use std::sync::mpsc::channel;
use std::thread;

use anyhow::Result;

use super::{App, Job, JobDone, Snapshot, file_name};
use crate::repo::Theme;
use crate::{actions, apps, ghostty, wallpaper};

impl App {
    pub(super) fn applied_theme_obj(&self) -> Option<Theme> {
        let current = self.applied_theme.as_deref()?;
        self.themes
            .iter()
            .find(|t| t.ghostty_name == current)
            .cloned()
    }

    pub(super) fn push_undo(&mut self) {
        self.undo.push(Snapshot {
            config: self.saved_config.clone(),
            wallpaper: self.applied_wallpaper.clone(),
            theme: self.applied_theme_obj(),
        });
    }

    /// Applies the selected theme and/or wallpaper on a worker thread.
    pub(super) fn start_apply(&mut self, theme: bool, wallpaper: bool) {
        if self.busy() {
            return;
        }
        let t = self.themes[self.selected].clone();
        let index = self.wallpaper[self.selected];
        let wallpaper = if wallpaper {
            match t.wallpapers.get(index) {
                Some(path) => Some((index, path.clone())),
                None if !theme => {
                    self.status = format!("{} has no wallpapers.", t.name);
                    return;
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
            let step = job_theme
                .as_ref()
                .map_or(ConfigStep::Keep, ConfigStep::Theme);
            let _ = tx.send(run_job(&step, &apps, job_wallpaper.as_deref()));
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
    }

    /// Puts back what was applied before the last change.
    pub(super) fn start_undo(&mut self) {
        if self.busy() {
            return;
        }
        let Some(snapshot) = self.undo.pop() else {
            self.status = "Nothing to undo.".into();
            return;
        };
        self.live_due = None;
        let apps = self.settings.apps.clone();
        let (tx, rx) = channel();
        let s = snapshot.clone();
        thread::spawn(move || {
            let step = ConfigStep::Restore {
                config: s.config.as_deref(),
                theme: s.theme.as_ref(),
            };
            let _ = tx.send(run_job(&step, &apps, s.wallpaper.as_deref()));
        });
        self.status = "Undoing…".into();
        self.job = Some((Job::Undo(snapshot), rx));
    }

    pub(super) fn finish_job(&mut self, job: Job, done: JobDone) {
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
                    .map_or_else(|| "your previous theme".into(), |t| t.name);
                self.status = format!(
                    "↶ Back to {name}{}. ({} more to undo)",
                    done.apps,
                    self.undo.len()
                );
            }
        }
    }
}

/// What a job does to the Ghostty config.
enum ConfigStep<'a> {
    /// Leave it alone (setting only the wallpaper).
    Keep,
    /// Set this theme, and its app colors.
    Theme(&'a Theme),
    /// Undo: put this config text back (`None` removes the file), and these app colors.
    Restore {
        config: Option<&'a str>,
        theme: Option<&'a Theme>,
    },
}

/// Changes the config and apps, then the wallpaper. Runs on a worker thread.
fn run_job(step: &ConfigStep, apps: &[String], wallpaper: Option<&Path>) -> JobDone {
    let mut done = JobDone {
        apps: String::new(),
        wallpaper_fallback: None,
        error: None,
    };
    let result = (|| -> Result<()> {
        match step {
            ConfigStep::Keep => {}
            ConfigStep::Theme(theme) => done.apps = actions::theme_effects(theme, apps)?,
            ConfigStep::Restore { config, theme } => {
                ghostty::write_config(&ghostty::config_path(), *config)?;
                if let Some(theme) = theme {
                    let (ok, _) = apps::apply(theme, apps);
                    if !ok.is_empty() {
                        done.apps = format!(" (+ {})", ok.join(", "));
                    }
                }
            }
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
