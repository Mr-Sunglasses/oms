# omarchy-switch

Pick an [Omarchy](https://github.com/omacom/omarchy) theme for [Ghostty](https://ghostty.org) and a matching macOS wallpaper from one terminal UI, and apply both with one key.

![omarchy-switch showing Tokyo Night](docs/screenshot.png)

- **All 22 Omarchy themes**, each with a preview of its colors and its own wallpapers.
- **Live preview:** while you move through the list, Ghostty recolors to the selected theme. Quit without applying and your theme comes back.
- **Real wallpaper previews**, drawn as images right in the terminal.
- **One key applies both:** the Ghostty theme and the desktop wallpaper on every display.

macOS only. Themes come from [ghostty-omarchy-themes](https://github.com/Mr-Sunglasses/ghostty-omarchy-themes) and wallpapers from [omarchy-wallpapers](https://github.com/Mr-Sunglasses/omarchy-wallpapers).

![omarchy-switch showing Catppuccin Latte](docs/light.png)

## Install

You need [Rust](https://rustup.rs) and Git (`xcode-select --install`).

```sh
cargo install --git https://github.com/Mr-Sunglasses/omarchy-switch
```

Or from a clone:

```sh
git clone https://github.com/Mr-Sunglasses/omarchy-switch
cd omarchy-switch
cargo install --path .
```

## Use

```sh
omarchy-switch
```

The first run downloads the themes and wallpapers (about 100 MB) into `~/Library/Application Support/omarchy-switch` and installs the theme files into `~/.config/ghostty/themes`.

| Key | Action |
|---|---|
| <kbd>↑</kbd> <kbd>↓</kbd> or <kbd>j</kbd> <kbd>k</kbd> | Choose a theme |
| <kbd>←</kbd> <kbd>→</kbd> or <kbd>h</kbd> <kbd>l</kbd> | Choose one of its wallpapers |
| <kbd>Enter</kbd> | Apply the theme and the wallpaper |
| <kbd>t</kbd> | Apply only the theme |
| <kbd>w</kbd> | Apply only the wallpaper |
| <kbd>r</kbd> | Pick a random theme and wallpaper |
| <kbd>p</kbd> | Turn live preview on or off |
| <kbd>g</kbd> <kbd>G</kbd> | Jump to the first or last theme |
| <kbd>q</kbd> or <kbd>Esc</kbd> | Quit |

A green dot marks the theme and wallpaper that are applied now.

### From scripts

```sh
omarchy-switch list                      # themes and how many wallpapers each has
omarchy-switch apply tokyo-night         # theme + its first wallpaper
omarchy-switch apply "Tokyo Night" 3     # theme + its 3rd wallpaper
omarchy-switch apply nord random         # theme + a random wallpaper
omarchy-switch update                    # download new themes and wallpapers
```

To use your own checkouts of the two repos, pass `--themes <dir>` and `--wallpapers <dir>`.

## How it works

- **Theme:** sets the `theme = Omarchy …` line in your Ghostty config and leaves the rest of the file alone. It edits the file that sets the theme now, or `~/.config/ghostty/config` if none does. It then sends Ghostty `SIGUSR2`, which makes Ghostty reload its config and recolor every window.
- **Wallpaper:** uses macOS's `NSWorkspace` wallpaper API, so it needs no extra permissions. It sets the wallpaper for the current Space on each display.
- **Preview:** wallpapers are drawn with the Kitty graphics protocol, which Ghostty supports. Other terminals fall back to colored blocks.

Notes:

- If your config uses `theme = light:…,dark:…`, applying a theme replaces it with a single theme.
- The Omarchy themes also recolor Ghostty's app icon. Icon changes need a full restart of Ghostty (quit and reopen).
- Tested with Ghostty 1.3 on macOS.

## Credits

Themes and wallpapers come from [Omarchy](https://github.com/omacom/omarchy) by David Heinemeier Hansson and its contributors, and from the original theme authors and artists. Released under the [MIT License](LICENSE).
