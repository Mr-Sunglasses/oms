//! Downloading the repos: the themes in full, the wallpapers as just their
//! previews (`.previews/<theme>/<file>.jpg`) at first, with each theme's full
//! pictures fetched the first time one of them is used.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};

/// Folder of the wallpaper repo with a small JPEG of every wallpaper.
pub(super) const PREVIEWS_DIR: &str = ".previews";

/// `git clone` with a progress bar on the terminal. `previews_only`: check
/// out just the wallpaper previews; full pictures come later, per theme.
pub(super) fn clone_with_progress(
    url: &str,
    dir: &Path,
    label: &str,
    previews_only: bool,
) -> Result<()> {
    use std::io::{IsTerminal, Read, Write};
    let tty = std::io::stderr().is_terminal();
    let partial = dir.with_extension("partial");
    let _ = fs::remove_dir_all(&partial);
    let lazy: &[&str] = if previews_only {
        &["--filter=blob:none", "--sparse"]
    } else {
        &[]
    };
    let mut child = Command::new("git")
        .args(["clone", "--depth", "1", "--progress"])
        .args(lazy)
        .arg(url)
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
    if previews_only && let Err(e) = check_out_previews(&partial, tty, label) {
        let _ = fs::remove_dir_all(&partial);
        return Err(e);
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

/// Checks out the previews (git downloads them now, without a progress bar,
/// so show a spinner), or everything if this repo has no previews.
pub(super) fn check_out_previews(repo: &Path, tty: bool, label: &str) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["sparse-checkout", "set", PREVIEWS_DIR])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("running git")?;
    let frames = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let mut frame = 0;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if tty {
            eprint!(
                "\r  {label:<11} {} getting previews{:28}",
                frames[frame % frames.len()],
                ""
            );
            let _ = std::io::stderr().flush();
        }
        frame += 1;
        std::thread::sleep(std::time::Duration::from_millis(80));
    };
    if !status.success() {
        bail!("downloading the wallpaper previews failed");
    }
    if !repo.join(PREVIEWS_DIR).exists() {
        // An older wallpaper repo without previews: take all of it.
        git(Some(repo), &["sparse-checkout", "disable"])?;
    }
    Ok(())
}

/// The small preview of a wallpaper in the wallpaper repo, if it has one.
pub fn preview_of(path: &Path) -> Option<PathBuf> {
    let theme_dir = path.parent()?;
    let preview = theme_dir
        .parent()?
        .join(PREVIEWS_DIR)
        .join(theme_dir.file_name()?)
        .join(format!("{}.jpg", path.file_name()?.to_string_lossy()));
    preview.exists().then_some(preview)
}

/// Makes sure a wallpaper is on disk, downloading its theme's full pictures
/// if only their previews are there yet.
pub fn ensure_local(path: &Path) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if preview_of(path).is_some()
        && let Some(theme_dir) = path.parent()
        && let (Some(repo), Some(slug)) = (theme_dir.parent(), theme_dir.file_name())
    {
        // On a full checkout this fails and changes nothing; the file is
        // then really missing, which is reported below.
        let _ = git(
            Some(repo),
            &["sparse-checkout", "add", &slug.to_string_lossy()],
        );
    }
    if path.exists() {
        Ok(())
    } else {
        bail!("{} not found", path.display())
    }
}

/// Whether the wallpapers are a previews-first (sparse) checkout.
pub fn is_lazy(wallpapers: &Path) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(wallpapers)
        .args(["config", "--bool", "core.sparseCheckout"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "true")
}

pub(super) fn head(dir: &Path) -> Option<Vec<u8>> {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .map(|o| o.stdout)
}

/// Runs git without printing anything (it may run behind the TUI).
pub(super) fn git(dir: Option<&Path>, args: &[&str]) -> Result<()> {
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
