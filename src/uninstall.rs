//! `oms uninstall`: remove oms and what it set up.

use std::fs;
use std::io::{BufRead, Write};

use anyhow::Result;

use crate::settings::{Settings, data_dir};
use crate::{apps, daemon, ghostty};

pub fn run(args: &[String]) -> Result<()> {
    let all = args.iter().any(|a| a == "--all");
    let yes = args.iter().any(|a| a == "--yes" || a == "-y");
    let exe = std::env::current_exe()?.canonicalize()?;
    let settings = Settings::load();

    println!("This removes:");
    println!("  - the oms command ({})", exe.display());
    println!(
        "  - downloaded themes, wallpapers and settings ({})",
        data_dir().display()
    );
    println!("  - the background agent (light/dark switching, rotation)");
    let others = daemon::other_agents();
    if !others.is_empty() {
        let labels: Vec<&str> = others.iter().map(|a| a.label.as_str()).collect();
        println!("  - other oms agents ({})", labels.join(", "));
    }
    if !settings.apps.is_empty() {
        println!(
            "  - app themes set up by oms ({})",
            settings.apps.join(", ")
        );
    }
    if all {
        println!("  - the Omarchy theme files in Ghostty's themes folder");
    } else {
        println!(
            "It keeps your Ghostty config and the Omarchy theme files (add --all to remove those too)."
        );
    }
    if !yes && !confirm("Continue? [y/N] ")? {
        println!("Nothing removed.");
        return Ok(());
    }

    daemon::uninstall()?;
    for agent in &others {
        daemon::remove_other(agent)?;
    }
    for app in &settings.apps {
        apps::remove(app)?;
    }
    if all {
        let removed = ghostty::remove_themes()?;
        println!("Removed {removed} Omarchy theme files.");
        if ghostty::current_theme(&ghostty::config_path()).is_some_and(|t| t.contains("Omarchy ")) {
            println!("Your Ghostty config still names an Omarchy theme; change `theme =` in it.");
        }
    }
    let _ = fs::remove_dir_all(data_dir());
    // The running binary can be removed; macOS keeps it alive until we exit.
    fs::remove_file(&exe)?;
    println!("oms is uninstalled. Thanks for trying it!");
    Ok(())
}

fn confirm(prompt: &str) -> Result<bool> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut answer = String::new();
    std::io::stdin().lock().read_line(&mut answer)?;
    Ok(matches!(answer.trim(), "y" | "Y" | "yes"))
}
