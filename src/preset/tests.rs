use super::*;

#[test]
fn splits_the_preset_into_named_sections() {
    for config in [MACOS_CONFIG, LINUX_CONFIG] {
        let ids: Vec<String> = sections_of(config).into_iter().map(|s| s.id).collect();
        for id in ["theme", "font", "window", "shader", "keybindings"] {
            assert!(ids.contains(&id.to_string()), "missing {id}: {ids:?}");
        }
        // Every section has a short name, so `--only` lists stay the same.
        for s in sections_of(config) {
            assert!(
                SECTION_IDS.iter().any(|(_, id)| *id == s.id),
                "no short name for {:?}",
                s.title
            );
        }
    }
}

#[test]
fn linux_preset_has_nothing_macos_only() {
    let settings: Vec<(String, String)> = LINUX_CONFIG.lines().filter_map(setting).collect();
    for (key, value) in &settings {
        assert!(!key.starts_with("macos-"), "{key} only works on macOS");
        // `cmd` is super on Linux, which the desktop takes.
        assert!(
            !(key == "keybind" && (value.contains("cmd+") || value.contains("super+"))),
            "{value} uses super"
        );
        assert!(
            !(key == "keybind" && value.starts_with("global:")),
            "{value}: global keys belong to the desktop on Linux"
        );
        assert!(
            !["window-colorspace", "window-save-state", "alpha-blending"].contains(&key.as_str()),
            "{key} does nothing on Linux"
        );
    }
    // No binding takes over one of Ghostty's Linux defaults.
    for taken in [
        "ctrl+shift+a",
        "ctrl+shift+left",
        "ctrl+shift+up",
        "ctrl+shift+w",
    ] {
        assert!(
            !settings
                .iter()
                .any(|(k, v)| k == "keybind" && trigger(v).starts_with(taken)),
            "{taken} is a Ghostty default on Linux"
        );
    }
}

#[test]
fn install_keeps_includes_and_turns_off_omarchys_colors() {
    let omarchy = "config-file = ?\"~/.local/state/omarchy/current/theme/ghostty.conf\"\nfont-size = 9\nasync-backend = epoll\nconfig-file = mine.conf\n";
    let text = with_kept(LINUX_CONFIG, omarchy);
    assert!(text.contains("\nasync-backend = epoll\n"));
    assert!(text.contains("\nconfig-file = mine.conf\n"));
    assert!(text.contains("\n# config-file = ?\"~/.local/state/omarchy"));
    assert!(!ghostty::has_theme_include(&text));
    assert!(!text.contains("font-size = 9"));
    assert_eq!(with_kept(LINUX_CONFIG, "font-size = 9\n"), LINUX_CONFIG);
    // An include oms already turned off is kept, still off.
    let off = ghostty::with_theme(omarchy, "Omarchy Nord");
    let again = with_kept(LINUX_CONFIG, &off);
    assert!(again.contains("\n# config-file = ?\"~/.local/state/omarchy"));
    assert!(!ghostty::has_theme_include(&again));
}

#[test]
fn merging_replaces_settings_and_keeps_others() {
    let font = sections_of(MACOS_CONFIG)
        .into_iter()
        .find(|s| s.id == "font")
        .unwrap();
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
    let keys = sections_of(MACOS_CONFIG)
        .into_iter()
        .find(|s| s.id == "keybindings")
        .unwrap();
    let user = "keybind = cmd+shift+r=reload_config\nkeybind = ctrl+a=select_all\n";
    let merged = merge_section(user, &keys);
    assert!(
        merged.contains(
            "# (replaced by oms preset: keybindings) keybind = cmd+shift+r=reload_config"
        )
    );
    assert!(merged.contains("\nkeybind = ctrl+a=select_all"));
}
