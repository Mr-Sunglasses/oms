//! Reading and setting the macOS desktop picture.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow};
use objc2::MainThreadMarker;
use objc2::runtime::AnyObject;
use objc2_app_kit::{NSScreen, NSWorkspace};
use objc2_foundation::{NSDictionary, NSString, NSURL};
use plist::{Dictionary, Value};

const IMAGE_PROVIDER: &str = "com.apple.wallpaper.choice.image";

/// Sets the wallpaper on every display and every Space, and makes it the
/// default for new Spaces. Falls back to the current Space only when the
/// wallpaper store can't be used (macOS before 14).
pub fn set(path: &Path) -> Result<()> {
    let path = path
        .canonicalize()
        .with_context(|| format!("{} not found", path.display()))?;
    match set_in_store(&path) {
        Ok(()) => Ok(()),
        Err(_) => set_current_space(&path),
    }
}

/// The public API: only changes the current Space on each display.
fn set_current_space(path: &Path) -> Result<()> {
    let mtm = MainThreadMarker::new().context("wallpaper must be set from the main thread")?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()));
    let workspace = NSWorkspace::sharedWorkspace();
    let options = NSDictionary::<NSString, AnyObject>::new();
    for screen in NSScreen::screens(mtm).iter() {
        unsafe { workspace.setDesktopImageURL_forScreen_options_error(&url, &screen, &options) }
            .map_err(|e| anyhow!("could not set wallpaper: {}", e.localizedDescription()))?;
    }
    Ok(())
}

fn store_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_default()
        .join("com.apple.wallpaper/Store/Index.plist")
}

/// macOS 14+ keeps each Space's and display's wallpaper in WallpaperAgent's
/// store. Points every desktop entry at `path`, then restarts the agent so it
/// reloads the store.
fn set_in_store(path: &Path) -> Result<()> {
    let store = store_path();
    let mut root = Value::from_file(&store).context("reading the wallpaper store")?;
    let dict = root
        .as_dictionary_mut()
        .context("unexpected wallpaper store")?;

    let mut config = Dictionary::new();
    config.insert("type".into(), "imageFile".into());
    let mut url = Dictionary::new();
    url.insert(
        "relative".into(),
        format!("file://{}", encode_path(path)).into(),
    );
    config.insert("url".into(), url.into());
    let mut config_bytes = Vec::new();
    plist::to_writer_binary(&mut config_bytes, &Value::from(config))?;

    let mut entries = Vec::new();
    for (key, value) in dict.iter_mut() {
        match key.as_str() {
            "SystemDefault" | "AllSpacesAndDisplays" => entries.push(value),
            // Per display, and per Space (its default and each of its displays).
            "Displays" | "Spaces" => {
                if let Some(items) = value.as_dictionary_mut() {
                    for (_, item) in items.iter_mut() {
                        if key == "Displays" {
                            entries.push(item);
                            continue;
                        }
                        let Some(space) = item.as_dictionary_mut() else {
                            continue;
                        };
                        for (part, value) in space.iter_mut() {
                            match part.as_str() {
                                "Default" => entries.push(value),
                                "Displays" => {
                                    if let Some(displays) = value.as_dictionary_mut() {
                                        entries.extend(displays.iter_mut().map(|(_, v)| v));
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    let now = plist::Date::from(SystemTime::now());
    let mut changed = 0;
    for entry in entries {
        if update_desktop(entry, &config_bytes, now) {
            changed += 1;
        }
    }
    if changed == 0 {
        return Err(anyhow!("no desktops found in the wallpaper store"));
    }

    // Write next to the store and rename, so the agent never reads half a file.
    let tmp = store.with_extension("plist.oms");
    root.to_file_binary(&tmp)?;
    fs::rename(&tmp, &store)?;
    // launchd restarts the agent right away, and it picks up the new store.
    Command::new("killall").arg("WallpaperAgent").output()?;
    Ok(())
}

/// Points one `{Desktop: {Content: {Choices: [...]}}}` entry at the image.
fn update_desktop(entry: &mut Value, config: &[u8], now: plist::Date) -> bool {
    let Some(desktop) = entry
        .as_dictionary_mut()
        .and_then(|e| e.get_mut("Desktop"))
        .and_then(Value::as_dictionary_mut)
    else {
        return false;
    };
    desktop.insert("LastSet".into(), Value::Date(now));
    let Some(content) = desktop
        .get_mut("Content")
        .and_then(Value::as_dictionary_mut)
    else {
        return false;
    };
    let Some(choice) = content
        .get_mut("Choices")
        .and_then(Value::as_array_mut)
        .and_then(|c| c.first_mut())
        .and_then(Value::as_dictionary_mut)
    else {
        return false;
    };
    let was_image = choice.get("Provider").and_then(Value::as_string) == Some(IMAGE_PROVIDER);
    choice.insert("Provider".into(), IMAGE_PROVIDER.into());
    choice.insert("Configuration".into(), Value::Data(config.to_vec()));
    if !was_image {
        // Options of a dynamic or aerial wallpaper don't apply to a picture.
        content.insert(
            "EncodedOptionValues".into(),
            Value::Data(fill_screen_options()),
        );
    }
    true
}

/// `{values: {placement: {picker: {_0: {id: Crop}}}}}`: "Fill Screen".
fn fill_screen_options() -> Vec<u8> {
    let mut value = Value::from("Crop");
    for key in ["id", "_0", "picker", "placement", "values"] {
        let mut dict = Dictionary::new();
        dict.insert(key.into(), value);
        value = dict.into();
    }
    let mut bytes = Vec::new();
    let _ = plist::to_writer_binary(&mut bytes, &value);
    bytes
}

/// Percent-encodes a path for a file:// URL.
fn encode_path(path: &Path) -> String {
    let mut out = String::new();
    for b in path.to_string_lossy().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// The main display's current wallpaper.
pub fn current() -> Option<PathBuf> {
    let mtm = MainThreadMarker::new()?;
    let screen = NSScreen::mainScreen(mtm)?;
    let url = NSWorkspace::sharedWorkspace().desktopImageURLForScreen(&screen)?;
    Some(PathBuf::from(url.path()?.to_string()))
}
