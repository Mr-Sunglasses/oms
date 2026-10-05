# oms

Omarchy's themes, for your whole Mac. Pick one of the 22 [Omarchy](https://github.com/omacom/omarchy) themes and `oms` applies it everywhere at once: the [Ghostty](https://ghostty.org) theme, the wallpaper on every Space, and the colors of Neovim, btop, bat and tmux. It can also follow light and dark mode on its own.

**Website:** [oms.kanishkk.xyz](https://oms.kanishkk.xyz)

![oms showing Tokyo Night](docs/screenshot.png)

- **Live preview:** Ghostty recolors as you move through the list. Quit without applying and your theme comes back.
- **Wallpapers everywhere:** each theme's own wallpapers, set on every Space and display at once, with previews right in the terminal.
- **Light and dark:** pick a day theme and a night theme, and your Mac switches them when the appearance changes.
- **Your apps too:** Neovim, btop, bat, tmux and the macOS accent color can follow the theme.
- **Rotation:** move to the theme's next wallpaper on a timer.
- **Favorites, history and search:** star themes, see recent ones first, filter by name.
- **Your own wallpapers:** add pictures or folders to any theme.
- **Kanishk's Ghostty config:** install all of it, or just the parts you like.

macOS only. Themes come from [ghostty-omarchy-themes](https://github.com/Mr-Sunglasses/ghostty-omarchy-themes) and wallpapers from [omarchy-wallpapers](https://github.com/Mr-Sunglasses/omarchy-wallpapers).

![oms showing Catppuccin Latte](docs/light.png)

## Install

```sh
curl -fsSL https://kanishkk.xyz/oms | bash
```

Or straight from GitHub:

```sh
curl -fsSL https://raw.githubusercontent.com/Mr-Sunglasses/oms/main/install.sh | bash
```

This puts the `oms` command in `~/.local/bin`. It's a universal binary, so it runs on Apple silicon and Intel Macs. Git must be installed (`xcode-select --install`).

With [Rust](https://rustup.rs) you can also build it yourself:

```sh
cargo install --git https://github.com/Mr-Sunglasses/oms
```

Keep it current with `oms self-update`. The picker also tells you when a new version is out.

## The picker

```sh
oms
```

The first run downloads the themes and wallpapers (about 100 MB) into `~/Library/Application Support/omarchy-switch` and installs the theme files into `~/.config/ghostty/themes`. After that, `oms` checks for new themes, wallpapers and releases in the background every few hours.

| Key | Action |
|---|---|
| <kbd>↑</kbd> <kbd>↓</kbd> or <kbd>j</kbd> <kbd>k</kbd> | Choose a theme |
| <kbd>←</kbd> <kbd>→</kbd> or <kbd>h</kbd> <kbd>l</kbd> | Choose one of its wallpapers |
| <kbd>Enter</kbd> | Apply the theme and the wallpaper |
| <kbd>t</kbd> / <kbd>w</kbd> | Apply only the theme / only the wallpaper |
| <kbd>r</kbd> | Pick a random theme and wallpaper |
| <kbd>f</kbd> | Favorite: ★ themes sit at the top, · marks recent ones |
| <kbd>/</kbd> | Filter by name (<kbd>Esc</kbd> clears) |
| <kbd>L</kbd> / <kbd>D</kbd> | Use this theme and wallpaper in light / dark mode |
| <kbd>p</kbd> | Turn live preview on or off |
| <kbd>?</kbd> | Show all keys |
| <kbd>q</kbd> | Quit |

● marks what's applied now; ☀ and ☾ mark the light and dark themes.

## Light and dark mode

Pick a theme for each, in the picker with <kbd>L</kbd> and <kbd>D</kbd>, or from the command line:

```sh
oms auto catppuccin-latte tokyo-night     # light, dark
oms auto catppuccin-latte:2 tokyo-night:3 # with their 2nd and 3rd wallpapers
oms auto off
```

Ghostty switches its theme itself (`theme = light:…,dark:…`). A small background agent switches the wallpaper and app themes when macOS changes appearance. It's a LaunchAgent (`~/Library/LaunchAgents/xyz.kanishkk.oms.plist`) that only runs while light/dark switching or rotation is on, and it logs to `~/Library/Application Support/omarchy-switch/agent.log`. Applying a single theme turns light/dark switching off.

## Wallpapers

```sh
oms rotate 30m                       # the theme's next wallpaper every 30 minutes (or 2h, 1d)
oms rotate off
oms wallpapers add nord ~/Pictures/nordic     # your own pictures or folders
oms wallpapers list nord
oms wallpapers remove nord ~/Pictures/nordic
```

Your pictures appear after the theme's own ones, in the picker too. Wallpapers are set on every Space and every display, and become the default for new Spaces.

## Other apps

```sh
oms apps                        # what can follow the theme
oms apps on nvim btop bat tmux accent
oms apps off tmux               # stop, and remove what oms added
```

| App | What `oms` does |
|---|---|
| `nvim` | Writes `~/.config/nvim/lua/plugins/omarchy-theme.lua`, the Omarchy theme's colorscheme for [LazyVim](https://www.lazyvim.org). Restart Neovim to see it. |
| `btop` | Writes the `omarchy` color theme and selects it in `btop.conf`. |
| `bat` | Writes the `omarchy` theme, rebuilds bat's cache and sets `--theme` in bat's config. delta uses it too. |
| `tmux` | Writes `~/.config/tmux/omarchy-theme.conf`, sources it from your tmux.conf, and reloads running sessions. |
| `accent` | Sets the macOS accent color to the one closest to the theme's accent. |

The app themes come from [ghostty-omarchy-themes/apps](https://github.com/Mr-Sunglasses/ghostty-omarchy-themes/tree/main/apps): Omarchy's own Neovim and btop themes, plus bat and tmux themes made from the same colors.

## Kanishk's Ghostty config

`oms` ships with an opinionated Ghostty config, the one I use every day:
- the FiraCode Nerd Font with its nicer alternate glyphs, and extra line spacing;
- slight transparency and blur;
- a bar cursor with a smooth trail ([cursor shader](https://github.com/sahaj-b/ghostty-cursor-shaders), MIT);
- left Option working as Alt;
- 100 MB of scrollback;
- shell integration, including SSH fixes;
- keybindings for splits and tabs, plus a drop-down terminal on <kbd>Cmd</kbd>+<kbd>`</kbd>.

```sh
oms config install                         # all of it (keeps your theme)
oms config sections                        # list the parts
oms config install --only font,keybindings # just some parts, merged into your config
oms config show [section]                  # read it; every setting is commented
oms config restore                         # put your previous config back
```

Installing backs up your config first (`config.oms-<date>.bak`). With `--only`, each part goes into your config between `# >>> oms preset: <part>` markers, your own lines for the same settings are commented out, and running it again replaces the part instead of adding it twice.

Some window settings (transparency, blur, the shader) need a full restart of Ghostty. If the font is missing, `oms` tells you how to install it: `brew install --cask font-fira-code-nerd-font`.

## All commands

```text
oms                               open the picker
oms list | status                 list themes / show what's applied and switched on
oms apply <theme> [n|random]      apply a theme and one of its wallpapers
oms auto <light> <dark> | off     follow macOS light/dark mode
oms rotate <30m|2h|off>           rotate wallpapers
oms wallpapers add|remove|list    your own wallpapers
oms apps [on|off <app>...]        theme other apps
oms config ...                    Kanishk's Ghostty config
oms update                        download the latest themes and wallpapers
oms self-update                   update oms itself
oms uninstall [--all]             remove oms (--all: the Omarchy theme files too)
```

## How it works

- **Theme:** sets the `theme =` line in your Ghostty config and leaves the rest of the file alone. It edits the file that sets the theme now, or `~/.config/ghostty/config` if none does. It then sends Ghostty `SIGUSR2`, which makes Ghostty reload its config and recolor every window.
- **Wallpaper:** macOS's public API only changes the current Space, so `oms` updates the wallpaper settings file (`~/Library/Application Support/com.apple.wallpaper/Store/Index.plist`) for every Space and display, then restarts `WallpaperAgent` so it reloads them. This needs no extra permissions. On macOS before 14, which doesn't have that file, it changes the current Space only.
- **Preview:** wallpapers are drawn with the Kitty graphics protocol, which Ghostty supports. Other terminals fall back to colored blocks.

Notes:

- The Omarchy themes also recolor Ghostty's app icon. Icon changes need a full restart of Ghostty (quit and reopen).
- `OMS_DATA_DIR` moves where oms keeps its downloads and settings.
- Tested with Ghostty 1.3 on macOS 27.

## Releasing

Push a tag like `v0.6.0` (after bumping `version` in `Cargo.toml`) and GitHub Actions builds the universal binary and publishes the release. `oms self-update` and the install script pick it up.

Builds are signed and notarized automatically once these repository secrets exist (Settings → Secrets and variables → Actions). Until then they're unsigned, which is fine for the curl install:

| Secret | Value |
|---|---|
| `APPLE_CERTIFICATE` | Your "Developer ID Application" certificate as a `.p12`, base64-encoded (`base64 -i cert.p12 \| pbcopy`) |
| `APPLE_CERTIFICATE_PASSWORD` | The `.p12` password |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Kanishk Pachauri (TEAMID)` |
| `APPLE_ID` | Your Apple ID email |
| `APPLE_TEAM_ID` | Your 10-character team ID |
| `APPLE_APP_PASSWORD` | An [app-specific password](https://account.apple.com) for notarization |

The website lives in [`site/`](site) and deploys to GitHub Pages on every push that changes it. Add news to the "Notes" list in `site/index.html`. After themes change, rebuild its theme data and wallpaper thumbnails with `./site/build.py`.

## Credits

Themes and wallpapers come from [Omarchy](https://github.com/omacom/omarchy) by David Heinemeier Hansson and its contributors, and from the original theme authors and artists. Released under the [MIT License](LICENSE).
