//! oms: pick an Omarchy theme for Ghostty and a matching macOS
//! wallpaper from one TUI.

mod actions;
mod app;
mod apps;
mod cli;
mod completions;
mod daemon;
mod doctor;
mod ghostty;
mod images;
mod preset;
mod repo;
mod settings;
mod ui;
mod uninstall;
mod update;
mod wallpaper;

use std::io::{BufRead, IsTerminal, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
};
use ratatui::crossterm::execute;
use ratatui_image::picker::Picker;

use crate::app::App;
use crate::repo::Repos;
use crate::settings::Settings;

const USAGE: &str = "\
oms: Omarchy themes for Ghostty and matching macOS wallpapers

Usage:
  oms                               open the picker (press ? inside for keys)
  oms list                          list themes (★ favorites)
  oms apply <theme> [n|random]      apply a theme and one of its wallpapers
  oms status                        show what's applied and what's switched on

Light and dark:
  oms auto <light> <dark>           follow macOS light/dark mode with two themes
                                    (pick wallpapers with theme:n, e.g. nord:2)
  oms auto off                      stop following light/dark mode

Wallpapers:
  oms rotate <30m|2h|off>           change to the theme's next wallpaper on a timer
  oms wallpapers add <theme> <path>...      add your own pictures or folders
  oms wallpapers remove <theme> <path>...   remove them again
  oms wallpapers list <theme>               list a theme's wallpapers

Other apps:
  oms apps                          list apps that can follow the theme
  oms apps on <app>...              theme them too: nvim btop bat tmux accent
  oms apps off <app>...             stop theming them (and undo it)

Ghostty config:
  oms config install                use Kanishk's Ghostty config (backs up yours)
  oms config install --only <a,b>   just some sections (see `oms config sections`)
  oms config show [section]         print the config
  oms config restore                put your previous Ghostty config back

Maintenance:
  oms doctor                        check your setup and say what to fix
  oms update                        download the latest themes and wallpapers
  oms self-update                   update oms itself to the latest release
  oms completions <zsh|bash|fish>   tab completion for your shell
  oms uninstall [--all]             remove oms (--all: Omarchy theme files too)

Options:
  --themes <dir>       use a ghostty-omarchy-themes checkout
  --wallpapers <dir>   use an omarchy-wallpapers checkout
  -h, --help           show this help
  -V, --version        show the version

Themes can be given as a name or slug: \"Tokyo Night\", tokyo-night.";

/// The command line: the command and its arguments, plus `--themes` and
/// `--wallpapers` overrides.
struct Args {
    rest: Vec<String>,
    themes_dir: Option<PathBuf>,
    wallpapers_dir: Option<PathBuf>,
}

/// Reads the command line. `None` means `--help` or `--version` was handled.
fn parse_args() -> Result<Option<Args>> {
    let mut args = Args {
        rest: Vec::new(),
        themes_dir: None,
        wallpapers_dir: None,
    };
    let mut iter = std::env::args().skip(1);
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "-V" | "--version" => {
                println!("oms {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            "--themes" => {
                let dir = iter.next().context("--themes needs a directory")?;
                args.themes_dir = Some(PathBuf::from(dir));
            }
            "--wallpapers" => {
                let dir = iter.next().context("--wallpapers needs a directory")?;
                args.wallpapers_dir = Some(PathBuf::from(dir));
            }
            _ => args.rest.push(arg),
        }
    }
    Ok(Some(args))
}

fn main() -> Result<()> {
    let Some(Args {
        rest: args,
        themes_dir,
        wallpapers_dir,
    }) = parse_args()?
    else {
        return Ok(());
    };
    let arg = |i: usize| args.get(i).map(String::as_str);

    // Commands that don't need the theme repos.
    match arg(0) {
        Some("self-update" | "selfupdate") => return update::self_update().map(|_| ()),
        Some("self") if arg(1) == Some("update") => return update::self_update().map(|_| ()),
        Some("uninstall") => return uninstall::run(&args[1..]),
        Some("daemon") => return daemon::run(),
        Some("completions") => return completions::print(arg(1)),
        Some("doctor") => {
            let repos = Repos::existing(themes_dir.clone(), wallpapers_dir.clone());
            doctor::run_checks(repos.as_ref());
            return Ok(());
        }
        Some("config") if arg(1) != Some("install") => return preset::run(&args[1..]),
        _ => {}
    }

    let repos = Repos::locate(themes_dir, wallpapers_dir)?;
    ghostty::install_themes(&repos.themes)?;
    let mut settings = Settings::load();

    let result = match arg(0) {
        None => tui(&repos, settings),
        Some("list") if arg(1) == Some("--names") => {
            for t in repos.load(&settings)? {
                println!("{}", t.slug);
            }
            return Ok(());
        }
        Some("config") => preset::run(&args[1..]),
        Some("update") => {
            let changed = repos.update()?;
            ghostty::install_themes(&repos.themes)?;
            println!(
                "{}",
                if changed {
                    "Downloaded the latest themes and wallpapers."
                } else {
                    "Already up to date."
                }
            );
            Ok(())
        }
        Some("list") => {
            for t in repos.load(&settings)? {
                let star = if settings.is_favorite(&t.slug) {
                    "★"
                } else {
                    " "
                };
                println!(
                    "{star} {:<18} {:<20} {} wallpapers",
                    t.slug,
                    t.name,
                    t.wallpapers.len()
                );
            }
            Ok(())
        }
        Some("apply") => {
            let themes = repos.load(&settings)?;
            let theme = actions::find(&themes, arg(1).context("which theme? See `oms list`")?)?;
            let outcome = actions::apply_theme(theme, &mut settings)?;
            println!("{}.", outcome.message);
            if !theme.wallpapers.is_empty() {
                let index = actions::wallpaper_index(theme, arg(2), &settings)?;
                let path = actions::apply_wallpaper(theme, index, &mut settings)?;
                println!("Wallpaper: {}", path.display());
            }
            Ok(())
        }
        Some("status") => cli::status(&repos, &settings),
        Some("auto") => cli::auto(&repos, &mut settings, &args[1..]),
        Some("rotate") => cli::rotate(&mut settings, arg(1)),
        Some("apps") => cli::apps_command(&repos, &mut settings, &args[1..]),
        Some("wallpapers") => cli::wallpapers(&repos, &mut settings, &args[1..]),
        Some(other) => bail!("unknown command {other:?}\n\n{USAGE}"),
    };
    // After other commands, mention a new release (the picker shows its own).
    if arg(0).is_some()
        && let Some(version) = Settings::load().update_to_offer()
    {
        eprintln!("\noms {version} is out. Update with: oms self-update");
    }
    result
}

/// Starts this program again with the same arguments (after an update).
fn restart() -> Result<()> {
    let exe = std::env::current_exe()?;
    let err = std::process::Command::new(exe)
        .args(std::env::args().skip(1))
        .exec();
    Err(err.into())
}

/// Asks to update when a newer release is known. Returns true if it updated.
fn offer_update(settings: &mut Settings) -> Result<bool> {
    let Some(version) = settings.update_to_offer() else {
        return Ok(false);
    };
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Ok(false);
    }
    print!(
        "\x1b[1moms {version} is out\x1b[0m (you have {}). Update now? [Y/n] ",
        env!("CARGO_PKG_VERSION")
    );
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    if matches!(answer.trim().to_lowercase().as_str(), "" | "y" | "yes") {
        if update::self_update()? {
            return Ok(true);
        }
        // The remembered release wasn't newer after all; forget it.
        settings.latest_version = None;
        settings.save()?;
        Ok(false)
    } else {
        settings.skipped_version = Some(version.clone());
        settings.save()?;
        println!(
            "OK. You won't be asked about {version} again; update any time with `oms self-update`."
        );
        Ok(false)
    }
}

fn tui(repos: &Repos, mut settings: Settings) -> Result<()> {
    if offer_update(&mut settings)? {
        return restart();
    }
    let themes = repos.load(&settings)?;
    let mut terminal = ratatui::init();
    let _ = execute!(std::io::stdout(), EnableMouseCapture);
    let picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    let mut app = App::new(themes, settings, picker, repos);
    let result = run(&mut terminal, &mut app);
    let restored = app.restore();
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    result.and(restored)?;
    if app.update_requested && update::self_update()? {
        return restart();
    }
    Ok(())
}

fn run(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    while !app.quit {
        app.tick();
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(Duration::from_millis(40))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => app.handle_key(key),
                Event::Mouse(mouse) => app.handle_mouse(mouse),
                _ => {}
            }
        }
    }
    Ok(())
}
