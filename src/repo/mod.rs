//! The two source repos: Ghostty theme files and wallpapers sorted by theme.
//! How they're downloaded is in `download`.

mod download;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use self::download::{PREVIEWS_DIR, clone_with_progress, git, head};
pub use self::download::{ensure_local, is_lazy, preview_of};
use crate::settings::{Settings, data_dir};

pub const THEMES_URL: &str = "https://github.com/Mr-Sunglasses/ghostty-omarchy-themes";
pub const WALLPAPERS_URL: &str = "https://github.com/Mr-Sunglasses/omarchy-wallpapers";

/// A theme Omarchy renamed or replaced, so saved choices can follow it.
pub struct Renamed {
    pub old_slug: &'static str,
    pub old_name: &'static str,
    pub new_slug: &'static str,
    pub new_name: &'static str,
}

pub const RENAMED: &[Renamed] = &[Renamed {
    old_slug: "osaka-jade",
    old_name: "Osaka Jade",
    new_slug: "jade-horse",
    new_name: "Jade Horse",
}];

/// The slug a renamed theme has now.
pub fn renamed(slug: &str) -> Option<&'static str> {
    RENAMED
        .iter()
        .find(|r| r.old_slug == slug)
        .map(|r| r.new_slug)
}

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

    /// Exact match on the slug or either name, including the names it had
    /// before Omarchy renamed it.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.to_lowercase();
        let old = RENAMED
            .iter()
            .filter(|r| r.new_slug == self.slug)
            .flat_map(|r| {
                [
                    r.old_slug.to_string(),
                    r.old_name.to_string(),
                    format!("Omarchy {}", r.old_name),
                ]
            });
        [
            self.slug.clone(),
            self.name.clone(),
            self.ghostty_name.clone(),
        ]
        .into_iter()
        .chain(old)
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
                "\x1b[1mWelcome to oms!\x1b[0m Downloading the themes and wallpaper previews (once, about 20 MB)…\n"
            );
        }
        if !repos.themes.exists() {
            clone_with_progress(THEMES_URL, &repos.themes, "Themes", false)?;
        }
        if !repos.wallpapers.exists() {
            clone_with_progress(WALLPAPERS_URL, &repos.wallpapers, "Wallpapers", true)?;
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

/// A theme's wallpapers: the ones on disk, plus the ones only previewed so far.
fn theme_wallpapers(wallpapers: &Path, slug: &str) -> Vec<PathBuf> {
    let dir = wallpapers.join(slug);
    let mut found: BTreeSet<PathBuf> = fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| is_image(p))
        .collect();
    let previews = fs::read_dir(wallpapers.join(PREVIEWS_DIR).join(slug));
    for entry in previews.into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if let Some(original) = name.strip_suffix(".jpg") {
            let path = dir.join(original);
            if is_image(&path) {
                found.insert(path);
            }
        }
    }
    found.into_iter().collect()
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
    theme.wallpapers = theme_wallpapers(wallpapers, &theme.slug);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh, empty folder for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oms-test-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"").unwrap();
    }

    #[test]
    fn lists_downloaded_and_previewed_wallpapers() {
        let repo = scratch("list");
        touch(&repo.join("nord/1-a.jpg"));
        touch(&repo.join(".previews/nord/1-a.jpg.jpg"));
        // Only previewed so far, keeping its own extension.
        touch(&repo.join(".previews/nord/2-b.png.jpg"));
        touch(&repo.join(".previews/nord/notes.txt"));
        let found = theme_wallpapers(&repo, "nord");
        assert_eq!(
            found,
            [repo.join("nord/1-a.jpg"), repo.join("nord/2-b.png")]
        );

        assert_eq!(
            preview_of(&repo.join("nord/2-b.png")),
            Some(repo.join(".previews/nord/2-b.png.jpg"))
        );
        assert_eq!(preview_of(&repo.join("nord/3-c.jpg")), None);
        assert!(ensure_local(&repo.join("nord/1-a.jpg")).is_ok());
        assert!(ensure_local(&repo.join("nord/3-c.jpg")).is_err());
        let _ = fs::remove_dir_all(&repo);
    }

    /// The real flow against a local repo: clone just the previews, then
    /// download a theme's pictures when one is needed.
    #[test]
    fn downloads_a_theme_when_first_used() {
        let dir = scratch("lazy");
        let source = dir.join("source");
        touch(&source.join("nord/1-a.jpg"));
        touch(&source.join(".previews/nord/1-a.jpg.jpg"));
        touch(&source.join("white/1-w.png"));
        touch(&source.join(".previews/white/1-w.png.jpg"));
        let quiet = |dir: &Path, args: &[&str]| git(Some(dir), args).unwrap();
        quiet(&source, &["init", "-q"]);
        quiet(&source, &["config", "uploadpack.allowFilter", "true"]);
        quiet(&source, &["add", "-A"]);
        quiet(
            &source,
            &[
                "-c",
                "user.name=oms",
                "-c",
                "user.email=oms@example.com",
                "commit",
                "-qm",
                "x",
            ],
        );
        let wallpapers = dir.join("wallpapers");
        let url = format!("file://{}", source.display());
        git(
            None,
            &[
                "clone",
                "-q",
                "--depth",
                "1",
                "--filter=blob:none",
                "--sparse",
                &url,
                &wallpapers.to_string_lossy(),
            ],
        )
        .unwrap();
        download::check_out_previews(&wallpapers, false, "Wallpapers").unwrap();

        assert!(is_lazy(&wallpapers));
        let nord = wallpapers.join("nord/1-a.jpg");
        assert!(!nord.exists());
        assert_eq!(
            theme_wallpapers(&wallpapers, "nord"),
            std::slice::from_ref(&nord)
        );
        ensure_local(&nord).unwrap();
        assert!(nord.exists());
        assert!(!wallpapers.join("white/1-w.png").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn finds_renamed_themes_by_their_old_names() {
        let theme = Theme {
            name: "Jade Horse".into(),
            ghostty_name: "Omarchy Jade Horse".into(),
            slug: "jade-horse".into(),
            background: (0, 0, 0),
            foreground: (0, 0, 0),
            cursor: (0, 0, 0),
            palette: [(0, 0, 0); 16],
            accent: (0, 0, 0),
            wallpapers: Vec::new(),
            builtin_wallpapers: 0,
            apps_dir: PathBuf::new(),
        };
        for query in [
            "jade-horse",
            "Jade Horse",
            "osaka-jade",
            "Osaka Jade",
            "omarchy osaka jade",
        ] {
            assert!(theme.matches(query), "{query}");
        }
        assert!(!theme.matches("nord"));
        assert_eq!(renamed("osaka-jade"), Some("jade-horse"));
        assert_eq!(renamed("nord"), None);
    }
}
