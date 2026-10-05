//! omarchy-switch: pick an Omarchy theme for Ghostty and a matching macOS
//! wallpaper from one TUI.

mod app;
mod ghostty;
mod images;
mod repo;
mod ui;
mod wallpaper;

use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use ratatui_image::picker::Picker;

use crate::app::App;
use crate::repo::{Repos, Theme};

const USAGE: &str = "\
oms (omarchy-switch): Omarchy themes for Ghostty and matching macOS wallpapers

Usage:
  oms                          open the picker
  oms list                     list themes and wallpaper counts
  oms apply <theme> [n|random] apply a theme and its nth (default 1st) wallpaper
  oms update                   download the latest themes and wallpapers

Options:
  --themes <dir>       use a ghostty-omarchy-themes checkout
  --wallpapers <dir>   use an omarchy-wallpapers checkout
  -h, --help           show this help
  -V, --version        show the version

Themes can be given as a name or slug: \"Tokyo Night\", tokyo-night.";

fn main() -> Result<()> {
    let mut args = Vec::new();
    let (mut themes_dir, mut wallpapers_dir) = (None, None);
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("oms {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            "--themes" => {
                themes_dir = Some(PathBuf::from(
                    iter.next().context("--themes needs a directory")?,
                ))
            }
            "--wallpapers" => {
                wallpapers_dir = Some(PathBuf::from(
                    iter.next().context("--wallpapers needs a directory")?,
                ))
            }
            _ => args.push(arg),
        }
    }

    let repos = Repos::locate(themes_dir, wallpapers_dir)?;
    match args.first().map(String::as_str) {
        Some("update") => {
            repos.update()?;
            let changed = ghostty::install_themes(&repos.themes)?;
            println!("Up to date ({changed} theme files changed).");
            Ok(())
        }
        Some("list") => {
            for t in repos.load()? {
                println!(
                    "{:<18} {:<20} {} wallpapers",
                    t.slug,
                    t.name,
                    t.wallpapers.len()
                );
            }
            Ok(())
        }
        Some("apply") => {
            ghostty::install_themes(&repos.themes)?;
            apply(repos.load()?, args.get(1), args.get(2))
        }
        None => {
            ghostty::install_themes(&repos.themes)?;
            let themes = repos.load()?;
            let mut terminal = ratatui::init();
            let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
            let mut app = App::new(themes, picker);
            let result = run(&mut terminal, &mut app);
            let restored = app.restore();
            ratatui::restore();
            result.and(restored)
        }
        Some(other) => bail!("unknown command {other:?}\n\n{USAGE}"),
    }
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(40))?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            app.handle_key(key);
        }
    }
    Ok(())
}

fn apply(themes: Vec<Theme>, name: Option<&String>, which: Option<&String>) -> Result<()> {
    let name = name.context("which theme? Run `oms list` to see them")?;
    let theme = themes
        .iter()
        .find(|t| t.matches(name))
        .with_context(|| format!("no theme called {name:?}"))?;

    ghostty::set_theme(&ghostty::config_path(), &theme.ghostty_name)?;
    println!("Ghostty theme: {}", theme.ghostty_name);

    let count = theme.wallpapers.len();
    if count == 0 {
        return Ok(());
    }
    let index = match which.map(String::as_str) {
        None => 0,
        Some("random") => {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .subsec_nanos() as usize
                % count
        }
        Some(n) => match n.parse::<usize>() {
            Ok(n) if (1..=count).contains(&n) => n - 1,
            _ => bail!("{} has wallpapers 1 to {count}", theme.name),
        },
    };
    let path = &theme.wallpapers[index];
    wallpaper::set(path)?;
    println!("Wallpaper: {}", path.display());
    Ok(())
}
