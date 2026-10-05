//! `oms config`: install Kanishk's opinionated Ghostty config (all of it, or
//! just some sections), or undo it.

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

/// Section titles in the preset and the short names used with `--only`.
const SECTION_IDS: &[(&str, &str)] = &[
    ("Theme", "theme"),
    ("Font", "font"),
    ("Text colors & readability", "colors"),
    ("Window", "window"),
    ("Transparency", "transparency"),
    ("Cursor & mouse", "cursor"),
    ("Input", "input"),
    ("Splits", "splits"),
    ("Cursor trail shader", "shader"),
    ("Shell integration", "shell"),
    ("Scrollback", "scrollback"),
    ("Keybindings", "keybindings"),
];

/// Settings that may appear many times; the preset's lines are added to the
/// user's instead of replacing them. (`keybind` lines replace only a binding
/// for the same keys.)
const REPEATABLE: &[&str] = &["font-feature", "keybind"];

const DIVIDER: &str =
    "# ------------------------------------------------------------------------------";

pub struct Section {
    pub id: String,
    pub title: String,
    pub text: String,
}

/// Splits the preset at its section headers (a title between two dividers).
pub fn sections() -> Vec<Section> {
    let lines: Vec<&str> = CONFIG.lines().collect();
    let starts: Vec<usize> = (0..lines.len().saturating_sub(2))
        .filter(|&i| {
            lines[i] == DIVIDER && lines[i + 2] == DIVIDER && lines[i + 1].starts_with("#  ")
        })
        .collect();
    starts
        .iter()
        .enumerate()
        .map(|(n, &start)| {
            let end = starts.get(n + 1).copied().unwrap_or(lines.len());
            let title = lines[start + 1].trim_start_matches('#').trim().to_string();
            let id = SECTION_IDS
                .iter()
                .find(|(t, _)| *t == title)
                .map(|(_, id)| id.to_string())
                .unwrap_or_else(|| {
                    title
                        .to_lowercase()
                        .replace(|c: char| !c.is_alphanumeric(), "-")
                });
            Section {
                id,
                title,
                text: lines[start..end].join("\n").trim_end().to_string(),
            }
        })
        .collect()
}

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("install") => match args.get(1).map(String::as_str) {
            Some("--only") => {
                let ids: Vec<String> = args
                    .get(2)
                    .context("--only needs sections, e.g. --only font,keybindings (see `oms config sections`)")?
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                install_sections(&ids)
            }
            Some(other) => bail!("unknown option {other:?}; did you mean --only?"),
            None => install(),
        },
        Some("sections") => {
            for s in sections() {
                let note = if s.id == "theme" {
                    "  (oms manages the theme)"
                } else {
                    ""
                };
                println!("{:<14} {}{note}", s.id, s.title);
            }
            Ok(())
        }
        Some("show") => match args.get(1) {
            Some(id) => {
                let s = sections()
                    .into_iter()
                    .find(|s| &s.id == id)
                    .with_context(|| format!("no section {id:?} (see `oms config sections`)"))?;
                println!("{}", s.text);
                Ok(())
            }
            None => {
                print!("{CONFIG}");
                Ok(())
            }
        },
        None => {
            print!("{CONFIG}");
            Ok(())
        }
        Some("restore") => restore(),
        Some(other) => {
            bail!("unknown `oms config {other}`; use install, sections, show or restore")
        }
    }
}

/// Backs up the current config, writes the preset with the user's own theme,
/// adds the shader it uses, and reloads Ghostty.
fn install() -> Result<()> {
    let path = ghostty::config_path();
    let mut text = CONFIG.to_string();
    let current = ghostty::read_config(&path);
    if current.is_some() {
        // Keep the theme they already use.
        if let Some(theme) = ghostty::current_theme(&path) {
            text = ghostty::with_theme(&text, &theme);
        }
    }
    write_config(&path, current.as_deref(), &text)?;
    println!("Installed the oms Ghostty config at {}", path.display());
    write_shaders(&path)?;
    finish(true);
    Ok(())
}

/// Merges only the chosen sections into the user's config.
fn install_sections(ids: &[String]) -> Result<()> {
    let all = sections();
    let mut chosen = Vec::new();
    for id in ids {
        if id == "theme" {
            bail!("oms manages the theme; pick it in `oms` instead");
        }
        let section = all
            .iter()
            .find(|s| &s.id == id)
            .with_context(|| format!("no section {id:?} (see `oms config sections`)"))?;
        chosen.push(section);
    }

    let path = ghostty::config_path();
    let current = ghostty::read_config(&path);
    let mut text = current.clone().unwrap_or_default();
    for section in &chosen {
        text = merge_section(&text, section);
    }
    write_config(&path, current.as_deref(), &text)?;
    let names: Vec<&str> = chosen.iter().map(|s| s.id.as_str()).collect();
    println!("Added {} to {}", names.join(", "), path.display());
    if chosen.iter().any(|s| s.text.contains("custom-shader")) {
        write_shaders(&path)?;
    }
    finish(chosen.iter().any(|s| s.text.contains("font-family")));
    Ok(())
}

/// Adds a section between `# >>> oms preset: id` / `# <<< oms preset: id`
/// markers (replacing an earlier copy), and comments out the user's own lines
/// for the same settings so the preset's take effect.
fn merge_section(text: &str, section: &Section) -> String {
    let start = format!("# >>> oms preset: {}", section.id);
    let end = format!("# <<< oms preset: {}", section.id);

    // Drop a previous copy of this section.
    let mut lines: Vec<String> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        if line == start {
            inside = true;
        } else if line == end {
            inside = false;
        } else if !inside {
            lines.push(line.to_string());
        }
    }

    let settings: Vec<(String, String)> = section.text.lines().filter_map(setting).collect();
    let replaced = |key: &str, value: &str| {
        settings.iter().any(|(k, v)| {
            k == key
                && (!REPEATABLE.contains(&key)
                    || (key == "keybind" && trigger(v) == trigger(value)))
        })
    };
    for line in lines.iter_mut() {
        if let Some((key, value)) = setting(line)
            && replaced(&key, &value)
        {
            *line = format!("# (replaced by oms preset: {}) {line}", section.id);
        }
    }

    let mut out = lines.join("\n").trim_end().to_string();
    out.push_str(&format!("\n\n{start}\n{}\n{end}\n", section.text));
    out
}

/// `key = value` of an active setting line.
fn setting(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    Some((key.trim().to_string(), value.trim().to_string()))
}

/// The keys of a keybind value: `cmd+shift+r=new_split:right` -> `cmd+shift+r`.
fn trigger(value: &str) -> &str {
    // Triggers never contain '=' (that key is spelled `equal`); actions can.
    value
        .split_once('=')
        .map(|(t, _)| t.trim())
        .unwrap_or(value)
}

/// Backs up the old config and writes the new one.
fn write_config(path: &Path, old: Option<&str>, text: &str) -> Result<()> {
    fs::create_dir_all(path.parent().context("bad config path")?)?;
    if let Some(old) = old {
        let backup = backup_path(path)?;
        fs::write(&backup, old)?;
        println!("Backed up your config to {}", backup.display());
    }
    fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}

fn finish(check_font: bool) {
    ghostty::reload();
    println!();
    if check_font && !font_installed() {
        println!("The config uses the {FONT} font, which isn't installed. Install it with:");
        println!();
        println!("  brew install --cask font-fira-code-nerd-font");
        println!();
    }
    println!("Ghostty has reloaded it. Quit and reopen Ghostty (cmd+q) for the window");
    println!("settings (transparency, blur, shader, icon) to take effect.");
    println!("Undo with: oms config restore");
}

fn write_shaders(config: &Path) -> Result<()> {
    let dir = config.parent().context("bad config path")?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_the_preset_into_named_sections() {
        let ids: Vec<String> = sections().into_iter().map(|s| s.id).collect();
        for id in ["theme", "font", "window", "shader", "keybindings"] {
            assert!(ids.contains(&id.to_string()), "missing {id}: {ids:?}");
        }
    }

    #[test]
    fn merging_replaces_settings_and_keeps_others() {
        let font = sections().into_iter().find(|s| s.id == "font").unwrap();
        let user = "font-size = 14\ntheme = Nord\nfont-feature = calt\n";
        let merged = merge_section(user, &font);
        assert!(merged.contains("# (replaced by oms preset: font) font-size = 14"));
        assert!(merged.contains("theme = Nord"));
        assert!(merged.contains("font-feature = calt")); // repeatable: kept
        assert!(merged.contains("# >>> oms preset: font"));
        // Merging again replaces the block instead of adding a second one.
        let again = merge_section(&merged, &font);
        assert_eq!(again.matches("# >>> oms preset: font").count(), 1);
    }

    #[test]
    fn keybinds_replace_only_the_same_keys() {
        let keys = sections()
            .into_iter()
            .find(|s| s.id == "keybindings")
            .unwrap();
        let user = "keybind = cmd+shift+r=reload_config\nkeybind = ctrl+a=select_all\n";
        let merged = merge_section(user, &keys);
        assert!(merged.contains(
            "# (replaced by oms preset: keybindings) keybind = cmd+shift+r=reload_config"
        ));
        assert!(merged.contains("\nkeybind = ctrl+a=select_all"));
    }
}
