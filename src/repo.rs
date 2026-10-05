//! The two source repos: Ghostty theme files and wallpapers sorted by theme.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::settings::{Settings, data_dir};

pub const THEMES_URL: &str = "https://github.com/Mr-Sunglasses/ghostty-omarchy-themes";
pub const WALLPAPERS_URL: &str = "https://github.com/Mr-Sunglasses/omarchy-wallpapers";

pub type Rgb = (u8, u8, u8);

#[derive(Clone)]
pub struct Theme {
    /// Display name, e.g. "Tokyo Night".
    pub name: String,
    /// Ghostty theme name, e.g. "Omarchy Tokyo Night".
    pub ghostty_name: String,
    /// Omarchy theme id, which is also the wallpaper folder, e.g. "tokyo-night".
    pub slug: String,
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub palette: [Rgb; 16],
    /// The theme's accent (its split divider color).
    pub accent: Rgb,
    /// Omarchy's wallpapers first, then the user's own.
    pub wallpapers: Vec<PathBuf>,
    /// How many of `wallpapers` come from Omarchy.
    pub builtin_wallpapers: usize,
    /// Matching Neovim, btop, bat and tmux themes.
    pub apps_dir: PathBuf,
}

impl Theme {
    pub fn is_light(&self) -> bool {
        let (r, g, b) = self.background;
        0.299 * f32::from(r) + 0.587 * f32::from(g) + 0.114 * f32::from(b) > 128.0
    }

    pub fn is_custom_wallpaper(&self, index: usize) -> bool {
        index >= self.builtin_wallpapers
    }

    /// Case-insensitive substring match on the name or slug, for filtering.
    pub fn contains(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        self.name.to_lowercase().contains(&q) || self.slug.contains(&q)
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        [&self.slug, &self.name, &self.ghostty_name]
            .iter()
            .any(|s| s.to_lowercase() == q)
    }
}

#[derive(Clone)]
pub struct Repos {
    pub themes: PathBuf,
    pub wallpapers: PathBuf,
}

impl Repos {
    fn paths(themes: Option<PathBuf>, wallpapers: Option<PathBuf>) -> Self {
        let data = data_dir();
        Repos {
            themes: themes.unwrap_or_else(|| data.join("ghostty-omarchy-themes")),
            wallpapers: wallpapers.unwrap_or_else(|| data.join("omarchy-wallpapers")),
        }
    }

    /// The checkouts if they're already downloaded; never downloads.
    pub fn existing(themes: Option<PathBuf>, wallpapers: Option<PathBuf>) -> Option<Self> {
        let repos = Self::paths(themes, wallpapers);
        (repos.themes.exists() && repos.wallpapers.exists()).then_some(repos)
    }

    /// Uses the given checkouts, or downloads them into the app's data directory.
    pub fn locate(themes: Option<PathBuf>, wallpapers: Option<PathBuf>) -> Result<Self> {
        let repos = Self::paths(themes, wallpapers);
        let missing = !repos.themes.exists() || !repos.wallpapers.exists();
        if missing {
            eprintln!(
                "\x1b[1mWelcome to oms!\x1b[0m Downloading the themes and wallpapers (once, about 100 MB)…\n"
            );
        }
        for (dir, url, label) in [
            (&repos.themes, THEMES_URL, "Themes"),
            (&repos.wallpapers, WALLPAPERS_URL, "Wallpapers"),
        ] {
            if !dir.exists() {
                clone_with_progress(url, dir, label)?;
            }
        }
        if missing {
            eprintln!();
        }
        Ok(repos)
    }

    /// Pulls the latest themes and wallpapers. Returns whether anything changed.
    pub fn update(&self) -> Result<bool> {
        let mut changed = false;
        for dir in [&self.themes, &self.wallpapers] {
            let before = head(dir);
            git(Some(dir), &["pull", "--quiet", "--ff-only"])?;
            changed |= head(dir) != before;
        }
        Ok(changed)
    }

    pub fn load(&self, settings: &Settings) -> Result<Vec<Theme>> {
        let dir = self.themes.join("themes");
        let mut themes = Vec::new();
        for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
            let path = entry?.path();
            let file_name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if file_name.starts_with("Omarchy ") {
                let mut theme = parse_theme(&path, &file_name, &self.wallpapers)?;
                theme.apps_dir = self.themes.join("apps").join(&theme.slug);
                if let Some(extra) = settings.custom_wallpapers.get(&theme.slug) {
                    theme.wallpapers.extend(expand_wallpapers(extra));
                }
                themes.push(theme);
            }
        }
        if themes.is_empty() {
            bail!("no themes found in {}", dir.display());
        }
        themes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(themes)
    }
}

/// `git clone` with a progress bar on the terminal.
fn clone_with_progress(url: &str, dir: &Path, label: &str) -> Result<()> {
    use std::io::{IsTerminal, Read, Write};
    let tty = std::io::stderr().is_terminal();
    let partial = dir.with_extension("partial");
    let _ = fs::remove_dir_all(&partial);
    let mut child = Command::new("git")
        .args(["clone", "--depth", "1", "--progress", url])
        .arg(&partial)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("running git (install it with `xcode-select --install`)")?;
    let mut stderr = child.stderr.take().context("no git output")?;
    let mut buffer = Vec::new();
    let mut chunk = [0u8; 512];
    let mut last = String::new();
    let draw = |percent: usize, detail: &str| {
        let filled = percent * 24 / 100;
        eprint!(
            "\r  {label:<11} [{}{}] {percent:>3}%  {detail:<20}",
            "█".repeat(filled),
            "░".repeat(24 - filled)
        );
        let _ = std::io::stderr().flush();
    };
    while let Ok(n) = stderr.read(&mut chunk) {
        if n == 0 {
            break;
        }
        buffer.extend_from_slice(&chunk[..n]);
        // git redraws its progress with '\r'; parse the newest line.
        while let Some(end) = buffer.iter().position(|&b| b == b'\r' || b == b'\n') {
            let line: Vec<u8> = buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line).trim().to_string();
            if let Some(rest) = line.strip_prefix("Receiving objects:") {
                let percent = rest
                    .trim()
                    .split('%')
                    .next()
                    .and_then(|p| p.trim().parse().ok())
                    .unwrap_or(0);
                let size = rest
                    .split(", ")
                    .nth(1)
                    .map(|s| s.split(" |").next().unwrap_or("").trim().to_string())
                    .unwrap_or_default();
                if tty {
                    draw(percent, &size);
                }
            }
            if !line.is_empty() {
                last = line;
            }
        }
    }
    let status = child.wait()?;
    if !status.success() {
        let _ = fs::remove_dir_all(&partial);
        bail!("downloading {url} failed: {last}");
    }
    fs::rename(&partial, dir)?;
    if tty {
        draw(100, "done");
        eprintln!();
    } else {
        eprintln!("{label}: downloaded");
    }
    Ok(())
}

/// Image files in `paths`, which can be files or folders.
pub fn expand_wallpapers(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for path in paths {
        if path.is_dir() {
            let mut files: Vec<PathBuf> = fs::read_dir(path)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| is_image(p))
                .collect();
            files.sort();
            out.extend(files);
        } else if is_image(path) {
            out.push(path.clone());
        }
    }
    out
}

pub fn is_image(path: &Path) -> bool {
    let ext = path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_lowercase();
    matches!(
        ext.as_str(),
        "jpg" | "jpeg" | "png" | "heic" | "webp" | "tiff" | "gif"
    )
}

fn head(dir: &Path) -> Option<Vec<u8>> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .map(|o| o.stdout)
}

/// Runs git without printing anything (it may run behind the TUI).
fn git(dir: Option<&Path>, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let out = cmd.args(args).output().context("running git")?;
    if !out.status.success() {
        bail!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

fn parse_theme(path: &Path, file_name: &str, wallpapers: &Path) -> Result<Theme> {
    let text = fs::read_to_string(path)?;
    let mut theme = Theme {
        name: file_name.trim_start_matches("Omarchy ").to_string(),
        ghostty_name: file_name.to_string(),
        slug: String::new(),
        background: (0, 0, 0),
        foreground: (255, 255, 255),
        cursor: (255, 255, 255),
        palette: [(128, 128, 128); 16],
        accent: (128, 128, 255),
        wallpapers: Vec::new(),
        builtin_wallpapers: 0,
        apps_dir: PathBuf::new(),
    };
    for line in text.lines() {
        if let Some(rest) = line.split("Omarchy theme '").nth(1) {
            theme.slug = rest.split('\'').next().unwrap_or_default().to_string();
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "background" => theme.background = hex(value).unwrap_or(theme.background),
            "foreground" => theme.foreground = hex(value).unwrap_or(theme.foreground),
            "cursor-color" => theme.cursor = hex(value).unwrap_or(theme.cursor),
            "split-divider-color" => theme.accent = hex(value).unwrap_or(theme.accent),
            "palette" => {
                if let Some((i, color)) = value.split_once('=')
                    && let (Ok(i), Some(color)) = (i.trim().parse::<usize>(), hex(color.trim()))
                    && i < 16
                {
                    theme.palette[i] = color;
                }
            }
            _ => {}
        }
    }
    if theme.slug.is_empty() {
        theme.slug = theme.name.to_lowercase().replace(' ', "-");
    }
    if let Ok(entries) = fs::read_dir(wallpapers.join(&theme.slug)) {
        theme.wallpapers = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| is_image(p))
            .collect();
        theme.wallpapers.sort();
    }
    theme.builtin_wallpapers = theme.wallpapers.len();
    Ok(theme)
}

fn hex(s: &str) -> Option<Rgb> {
    let s = s.trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let n = u32::from_str_radix(s, 16).ok()?;
    Some(((n >> 16) as u8, (n >> 8) as u8, n as u8))
}
