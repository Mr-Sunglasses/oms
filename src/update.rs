//! `oms self-update`: replace this binary with the latest GitHub release.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

const REPO: &str = "Mr-Sunglasses/oms";
const ASSET: &str = "oms-macos-universal.tar.gz";

/// The newer release's version, if there is one.
pub fn available() -> Option<String> {
    let tag = latest_tag().ok()?;
    let latest = tag.trim_start_matches('v');
    is_newer(latest, env!("CARGO_PKG_VERSION")).then(|| latest.to_string())
}

/// Returns whether a newer version was installed.
pub fn self_update() -> Result<bool> {
    let current = env!("CARGO_PKG_VERSION");
    let tag = latest_tag()?;
    let latest = tag.trim_start_matches('v');
    if !is_newer(latest, current) {
        println!("oms {current} is already the latest version.");
        return Ok(false);
    }

    let exe = std::env::current_exe()?.canonicalize()?;
    let dir = exe
        .parent()
        .context("can't find the oms install directory")?;
    let tmp = tempdir()?;
    let result = (|| -> Result<()> {
        println!("Downloading oms {latest}...");
        let base = format!("https://github.com/{REPO}/releases/download/{tag}/{ASSET}");
        let archive = tmp.join(ASSET);
        curl(&base, &archive)?;
        curl(&format!("{base}.sha256"), &tmp.join("sha256"))?;
        verify(&archive, &tmp.join("sha256"))?;
        run(Command::new("tar")
            .arg("-xzf")
            .arg(&archive)
            .arg("-C")
            .arg(&tmp))?;

        // Rename over the old binary: atomic, and safe while it's running.
        let new = tmp.join("oms");
        let staged = dir.join(".oms-update");
        fs::copy(&new, &staged)?;
        fs::set_permissions(&staged, fs::metadata(&exe)?.permissions())?;
        fs::rename(&staged, &exe)
            .with_context(|| format!("replacing {} (try again with sudo?)", exe.display()))?;
        Ok(())
    })();
    let _ = fs::remove_dir_all(&tmp);
    result?;
    println!("Updated oms {current} → {latest} ({}).", exe.display());
    // A running background agent would keep the old version until logout.
    crate::daemon::restart_if_running();
    Ok(true)
}

/// The newest release tag, from where /releases/latest redirects to.
pub fn latest_tag() -> Result<String> {
    let url = format!("https://github.com/{REPO}/releases/latest");
    let out = Command::new("curl")
        .args(["-fsSLI", "-o", "/dev/null", "-w", "%{url_effective}", &url])
        .output()
        .context("running curl")?;
    if !out.status.success() {
        bail!("couldn't reach GitHub to check for updates");
    }
    let effective = String::from_utf8_lossy(&out.stdout);
    match effective.rsplit_once("/tag/") {
        Some((_, tag)) if !tag.is_empty() => Ok(tag.trim().to_string()),
        _ => bail!("no releases found at {url}"),
    }
}

pub fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.parse().unwrap_or(0)).collect() };
    parse(latest) > parse(current)
}

fn tempdir() -> Result<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("oms-update-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn curl(url: &str, dest: &Path) -> Result<()> {
    run(Command::new("curl").args(["-fsSL", url, "-o"]).arg(dest))
        .with_context(|| format!("downloading {url}"))
}

fn verify(archive: &Path, sum_file: &Path) -> Result<()> {
    let expected = fs::read_to_string(sum_file)?;
    let expected = expected.split_whitespace().next().unwrap_or_default();
    let out = Command::new("shasum")
        .args(["-a", "256"])
        .arg(archive)
        .output()?;
    let actual = String::from_utf8_lossy(&out.stdout);
    if actual.split_whitespace().next() != Some(expected) {
        bail!("checksum mismatch for the download, not updating");
    }
    Ok(())
}

fn run(cmd: &mut Command) -> Result<()> {
    let status = cmd.status()?;
    if !status.success() {
        bail!("{cmd:?} failed");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_newer;

    #[test]
    fn compares_versions_numerically() {
        assert!(is_newer("0.10.0", "0.9.1"));
        assert!(is_newer("1.0.0", "0.2.0"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.9", "0.2.0"));
    }
}
