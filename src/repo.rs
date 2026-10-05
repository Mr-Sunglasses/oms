//! The two source repos: Ghostty theme files and wallpapers sorted by theme.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

pub const THEMES_URL: &str = "https://github.com/Mr-Sunglasses/ghostty-omarchy-themes";
pub const WALLPAPERS_URL: &str = "https://github.com/Mr-Sunglasses/omarchy-wallpapers";

pub type Rgb = (u8, u8, u8);

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
    pub wallpapers: Vec<PathBuf>,
}

impl Theme {
    pub fn is_light(&self) -> bool {
        let (r, g, b) = self.background;
        0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 128.0
    }

    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        [&self.slug, &self.name, &self.ghostty_name]
            .iter()
            .any(|s| s.to_lowercase() == q)
    }
}

pub struct Repos {
    pub themes: PathBuf,
    pub wallpapers: PathBuf,
}

impl Repos {
    /// Uses the given checkouts, or clones into the app's data directory.
    pub fn locate(themes: Option<PathBuf>, wallpapers: Option<PathBuf>) -> Result<Self> {
        let data = dirs::data_dir()
            .context("no data directory")?
            .join("omarchy-switch");
        let repos = Repos {
            themes: themes.unwrap_or_else(|| data.join("ghostty-omarchy-themes")),
            wallpapers: wallpapers.unwrap_or_else(|| data.join("omarchy-wallpapers")),
        };
        for (dir, url) in [
            (&repos.themes, THEMES_URL),
            (&repos.wallpapers, WALLPAPERS_URL),
        ] {
            if !dir.exists() {
                eprintln!("Downloading {url} ...");
                git(
                    None,
                    &[
                        "clone",
                        "--quiet",
                        "--depth",
                        "1",
                        url,
                        &dir.to_string_lossy(),
                    ],
                )?;
            }
        }
        Ok(repos)
    }

    /// Pulls the latest themes and wallpapers.
    pub fn update(&self) -> Result<()> {
        for dir in [&self.themes, &self.wallpapers] {
            eprintln!("Updating {} ...", dir.display());
            git(Some(dir), &["pull", "--quiet", "--ff-only"])?;
        }
        Ok(())
    }

    pub fn load(&self) -> Result<Vec<Theme>> {
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
                themes.push(parse_theme(&path, &file_name, &self.wallpapers)?);
            }
        }
        if themes.is_empty() {
            bail!("no themes found in {}", dir.display());
        }
        themes.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(themes)
    }
}

fn git(dir: Option<&Path>, args: &[&str]) -> Result<()> {
    let mut cmd = Command::new("git");
    if let Some(dir) = dir {
        cmd.arg("-C").arg(dir);
    }
    let status = cmd.args(args).status().context("running git")?;
    if !status.success() {
        bail!("git {} failed", args.join(" "));
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
        wallpapers: Vec::new(),
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
            .filter(|p| {
                let ext = p
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "heic")
            })
            .collect();
        theme.wallpapers.sort();
    }
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
