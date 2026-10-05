//! `oms config`: install Kanishk's opinionated Ghostty config, or undo it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::ghostty;

const CONFIG: &str = include_str!("../preset/config");
const SHADERS: &[(&str, &str)] = &[
    (
        "cursor_warp.glsl",
        include_str!("../preset/shaders/cursor_warp.glsl"),
    ),
    (
        "LICENSE.cursor-shaders",
        include_str!("../preset/shaders/LICENSE.cursor-shaders"),
    ),
];
const FONT: &str = "FiraCode Nerd Font Mono";

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("install") => install(),
        Some("show") | None => {
            print!("{CONFIG}");
            Ok(())
        }
        Some("restore") => restore(),
        Some(other) => bail!("unknown `oms config {other}`; use install, show or restore"),
    }
}

/// Backs up the current config, writes the preset with the user's own theme,
/// adds the shader it uses, and reloads Ghostty.
fn install() -> Result<()> {
    let path = ghostty::config_path();
    let dir = path.parent().context("bad config path")?.to_path_buf();
    fs::create_dir_all(&dir)?;

    let mut text = CONFIG.to_string();
    let current = ghostty::read_config(&path);
    if let Some(old) = &current {
        let backup = backup_path(&path)?;
        fs::write(&backup, old)?;
        println!("Backed up your config to {}", backup.display());
        // Keep the theme they already use.
        if let Some(theme) = ghostty::current_theme(&path) {
            text = ghostty::with_theme(&text, &theme);
        }
    }
    fs::write(&path, &text).with_context(|| format!("writing {}", path.display()))?;
    println!("Installed the oms Ghostty config at {}", path.display());

    let shaders = dir.join("shaders");
    fs::create_dir_all(&shaders)?;
    for (name, body) in SHADERS {
        let target = shaders.join(name);
        if fs::read_to_string(&target).is_ok_and(|t| t != *body) {
            let backup = backup_path(&target)?;
            fs::rename(&target, &backup)?;
            println!("Backed up {} to {}", target.display(), backup.display());
        }
        fs::write(&target, body)?;
    }

    ghostty::reload();
    println!();
    if !font_installed() {
        println!("The config uses the {FONT} font, which isn't installed. Install it with:");
        println!();
        println!("  brew install --cask font-fira-code-nerd-font");
        println!();
    }
    println!("Ghostty has reloaded it. Quit and reopen Ghostty (cmd+q) for the window");
    println!("settings (transparency, blur, shader, icon) to take effect.");
    println!("Undo with: oms config restore");
    Ok(())
}

/// Puts back the newest backup made by `install`.
fn restore() -> Result<()> {
    let path = ghostty::config_path();
    let name = path
        .file_name()
        .context("bad config path")?
        .to_string_lossy()
        .into_owned();
    let dir = path.parent().context("bad config path")?;
    let mut backups: Vec<PathBuf> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let n = p.file_name().unwrap_or_default().to_string_lossy();
            n.starts_with(&format!("{name}.oms-")) && n.ends_with(".bak")
        })
        .collect();
    backups.sort();
    let Some(latest) = backups.pop() else {
        bail!("no oms backups of {} found", path.display());
    };
    fs::copy(&latest, &path)?;
    ghostty::reload();
    println!("Restored {} from {}", path.display(), latest.display());
    Ok(())
}

/// `config` -> `config.oms-20261005-120000.bak`, next to the original.
fn backup_path(path: &Path) -> Result<PathBuf> {
    let out = Command::new("date").arg("+%Y%m%d-%H%M%S").output()?;
    let stamp = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let name = path.file_name().context("bad path")?.to_string_lossy();
    Ok(path.with_file_name(format!("{name}.oms-{stamp}.bak")))
}

fn font_installed() -> bool {
    let home = dirs::home_dir().unwrap_or_default();
    [home.join("Library/Fonts"), PathBuf::from("/Library/Fonts")]
        .iter()
        .filter_map(|d| fs::read_dir(d).ok())
        .flatten()
        .filter_map(|e| e.ok())
        .any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("FiraCodeNerdFontMono")
        })
}
