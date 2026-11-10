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
    // Intro animations, one downloaded and one only previewed.
    touch(&repo.join("nord/intros/1-a.mp4"));
    touch(&repo.join(".previews/nord/intros/2-b.mp4.jpg"));
    let (pictures, intros) = theme_wallpapers(&repo, "nord");
    assert_eq!(
        pictures,
        [repo.join("nord/1-a.jpg"), repo.join("nord/2-b.png")]
    );
    assert_eq!(
        intros,
        [
            repo.join("nord/intros/1-a.mp4"),
            repo.join("nord/intros/2-b.mp4")
        ]
    );
    assert_eq!(
        preview_of(&repo.join("nord/intros/2-b.mp4")),
        Some(repo.join(".previews/nord/intros/2-b.mp4.jpg"))
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
    touch(&source.join("nord/intros/1-a.mp4"));
    touch(&source.join(".previews/nord/intros/1-a.mp4.jpg"));
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
            // Don't depend on the developer's signing setup.
            "-c",
            "commit.gpgsign=false",
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
    let intro = wallpapers.join("nord/intros/1-a.mp4");
    assert!(!nord.exists());
    assert_eq!(
        theme_wallpapers(&wallpapers, "nord"),
        (vec![nord.clone()], vec![intro.clone()])
    );
    // Asking for the intro downloads the whole theme, its still included.
    ensure_local(&intro).unwrap();
    assert!(intro.exists());
    assert!(nord.exists());
    assert!(!wallpapers.join("white/1-w.png").exists());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn reads_hex_colors() {
    assert_eq!(hex("#2e3440"), Some((0x2e, 0x34, 0x40)));
    assert_eq!(hex("FFFFFF"), Some((255, 255, 255)));
    assert_eq!(hex("#fff"), None);
    assert_eq!(hex("#gggggg"), None);
}

#[test]
fn parses_a_ghostty_theme_file() {
    let dir = scratch("parse");
    let file = dir.join("themes/Omarchy Nord");
    touch(&file);
    fs::write(
        &file,
        "# Omarchy Nord\n\
         # Ported from Omarchy theme 'nord' (https://github.com/omacom/omarchy)\n\n\
         background = #2e3440\n\
         foreground = #d8dee9\n\
         cursor-color = #eceff4\n\
         palette = 1=#bf616a\n\
         palette = 15=#d8dee9\n\
         palette = 16=#000000\n\
         split-divider-color = #81a1c1\n",
    )
    .unwrap();
    let wallpapers = dir.join("wallpapers");
    touch(&wallpapers.join("nord/1-a.png"));
    touch(&wallpapers.join("nord/notes.txt"));

    let theme = parse_theme(&file, "Omarchy Nord", &wallpapers).unwrap();
    assert_eq!(theme.name, "Nord");
    assert_eq!(theme.ghostty_name, "Omarchy Nord");
    assert_eq!(theme.slug, "nord");
    assert_eq!(theme.background, (0x2e, 0x34, 0x40));
    assert_eq!(theme.cursor, (0xec, 0xef, 0xf4));
    assert_eq!(theme.accent, (0x81, 0xa1, 0xc1));
    assert_eq!(theme.palette[1], (0xbf, 0x61, 0x6a));
    assert_eq!(theme.palette[15], (0xd8, 0xde, 0xe9));
    // Out-of-range palette entries are ignored, untouched ones keep the default.
    assert_eq!(theme.palette[2], (128, 128, 128));
    assert_eq!(theme.wallpapers, [wallpapers.join("nord/1-a.png")]);
    assert_eq!(theme.builtin_wallpapers, 1);
    assert!(!theme.is_light());

    // Without the "Omarchy theme '…'" line the slug comes from the name.
    let plain = dir.join("themes/Omarchy Tokyo Night");
    fs::write(&plain, "background = #f0f0f0\n").unwrap();
    let theme = parse_theme(&plain, "Omarchy Tokyo Night", &wallpapers).unwrap();
    assert_eq!(theme.slug, "tokyo-night");
    assert!(theme.is_light());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn expands_wallpaper_files_and_folders() {
    let dir = scratch("expand");
    touch(&dir.join("pics/b.JPG"));
    touch(&dir.join("pics/a.webp"));
    touch(&dir.join("pics/readme.md"));
    touch(&dir.join("one.heic"));
    let found = expand_wallpapers(&[
        dir.join("pics"),
        dir.join("one.heic"),
        dir.join("notes.txt"),
        dir.join("missing.jpg"),
    ]);
    assert_eq!(
        found,
        [
            dir.join("pics/a.webp"),
            dir.join("pics/b.JPG"),
            dir.join("one.heic"),
            // A missing picture is kept; it may be on a drive that isn't mounted.
            dir.join("missing.jpg"),
        ]
    );
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
