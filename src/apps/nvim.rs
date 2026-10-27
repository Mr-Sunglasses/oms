//! Neovim: the theme's colorscheme plugin spec, written where a LazyVim-style
//! config loads it on start, and applied to every running Neovim right away.

use std::fs;
use std::os::unix::fs::FileTypeExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};

use super::{config_home, read_app_file, write};
use crate::repo::Theme;

/// Runs inside each Neovim, given the spec file: sets up the plugins
/// that take colors as options (Omarchy's aether themes share one plugin,
/// so its options must be re-applied), then switches the colorscheme. Errors
/// are swallowed so a plugin that isn't installed yet never shows one; it gets
/// installed on the next start.
const RELOAD_LUA: &str = r#"
return function(spec_file)
  local ok, specs = pcall(dofile, spec_file)
  if not ok or type(specs) ~= "table" then return end
  local scheme
  for _, spec in ipairs(specs) do
    if type(spec.opts) == "table" and spec.opts.colorscheme then scheme = spec.opts.colorscheme end
  end
  for _, spec in ipairs(specs) do
    if spec.name and type(spec.opts) == "table" and not spec.opts.colorscheme then
      pcall(function() require("lazy").load({ plugins = { spec.name } }) end)
      pcall(function() require(spec.name).setup(spec.opts) end)
    end
  end
  if scheme then pcall(vim.cmd.colorscheme, scheme) end
end
"#;

/// How long to wait for one Neovim to answer.
const ANSWER_WITHIN: Duration = Duration::from_secs(2);

pub fn theme_file() -> PathBuf {
    config_home().join("nvim/lua/plugins/omarchy-theme.lua")
}

pub fn apply(theme: &Theme) -> Result<()> {
    if !config_home().join("nvim/lua/plugins").exists() {
        bail!("needs a LazyVim-style config (~/.config/nvim/lua/plugins)");
    }
    let file = theme_file();
    write(&file, &read_app_file(theme, "neovim.lua")?)?;
    // Recolor running Neovims too; fine if there are none.
    reload_running(&file);
    Ok(())
}

fn reload_running(spec: &Path) {
    let sockets = sockets();
    if sockets.is_empty() {
        return;
    }
    let Some(nvim) = nvim_binary() else {
        return;
    };
    let script = crate::settings::data_dir().join("nvim-reload.lua");
    if write(&script, RELOAD_LUA).is_err() {
        return;
    }
    let expr = format!(
        "luaeval('dofile(_A[1])(_A[2])', [{}, {}])",
        vim_string(&script.to_string_lossy()),
        vim_string(&spec.to_string_lossy())
    );
    for socket in sockets {
        let child = Command::new(&nvim)
            .arg("--server")
            .arg(&socket)
            .args(["--remote-expr", &expr])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if let Ok(mut child) = child {
            let started = Instant::now();
            while matches!(child.try_wait(), Ok(None)) && started.elapsed() < ANSWER_WITHIN {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// A single-quoted Vim string.
fn vim_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The server sockets of running Neovims: `nvim.<pid>.0` in Neovim's run
/// directory, `$XDG_RUNTIME_DIR` or `$TMPDIR/nvim.$USER/<random>/`.
fn sockets() -> Vec<PathBuf> {
    let user = std::env::var("USER").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(runtime) = std::env::var_os("XDG_RUNTIME_DIR") {
        dirs.push(PathBuf::from(runtime));
    }
    if let Some(tmp) = temp_dir() {
        dirs.push(tmp.join(format!("nvim.{user}")));
    }
    let mut found = Vec::new();
    for dir in dirs {
        // The sockets sit in the directory itself or one level down.
        let subdirs = fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir());
        for dir in std::iter::once(dir.clone()).chain(subdirs) {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with("nvim.") && entry.file_type().is_ok_and(|t| t.is_socket()) {
                    found.push(entry.path());
                }
            }
        }
    }
    found
}

/// The per-user temp directory. launchd agents don't get `$TMPDIR`, so ask
/// the system for it.
fn temp_dir() -> Option<PathBuf> {
    if let Some(tmp) = std::env::var_os("TMPDIR") {
        return Some(PathBuf::from(tmp));
    }
    let out = Command::new("getconf")
        .arg("DARWIN_USER_TEMP_DIR")
        .output()
        .ok()?;
    let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!dir.is_empty()).then(|| PathBuf::from(dir))
}

fn nvim_binary() -> Option<PathBuf> {
    ["nvim", "/opt/homebrew/bin/nvim", "/usr/local/bin/nvim"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| {
            Command::new(p)
                .arg("--version")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_vim_strings() {
        assert_eq!(vim_string("/a/b c.lua"), "'/a/b c.lua'");
        assert_eq!(vim_string("it's"), "'it''s'");
    }
}
