#!/usr/bin/env python3
"""Rebuild the site's theme data (themes.json) and wallpaper thumbnails.

Run after the themes or wallpapers change:

    ./site/build.py                       # uses oms's downloaded copies
    ./site/build.py <themes-repo> <wallpapers-repo>

Uses macOS's `sips` to make the thumbnails.
"""

import json
import re
import subprocess
import sys
from pathlib import Path

SITE = Path(__file__).resolve().parent
DATA = Path.home() / "Library/Application Support/omarchy-switch"
THUMB_WIDTH = 1280


def main():
    themes_repo = Path(sys.argv[1]) if len(sys.argv) > 1 else DATA / "ghostty-omarchy-themes"
    wallpapers_repo = Path(sys.argv[2]) if len(sys.argv) > 2 else DATA / "omarchy-wallpapers"
    out = []
    for theme_file in sorted((themes_repo / "themes").glob("Omarchy *")):
        text = theme_file.read_text()
        slug = re.search(r"Omarchy theme '([^']+)'", text).group(1)
        values = dict(re.findall(r"^([a-z-]+) = (#[0-9a-fA-F]{6})$", text, re.M))
        palette = dict(re.findall(r"^palette = (\d+)=(#[0-9a-fA-F]{6})$", text, re.M))
        colors = json.loads((themes_repo / "apps" / slug / "colors.json").read_text())
        pictures = sorted(p for p in (wallpapers_repo / slug).iterdir() if p.suffix in (".jpg", ".png"))
        # Skip the Omarchy logo pictures when the theme has others.
        pictures = [p for p in pictures if p.stem != "omarchy"] or pictures
        thumb = SITE / "wallpapers" / f"{slug}.jpg"
        subprocess.run(
            ["sips", "-s", "format", "jpeg", "-s", "formatOptions", "70", "--resampleWidth",
             str(THUMB_WIDTH), str(pictures[0]), "--out", str(thumb)],
            check=True, capture_output=True,
        )
        out.append({
            "slug": slug,
            "name": theme_file.name.removeprefix("Omarchy "),
            "mode": colors["mode"],
            "bg": values["background"],
            "fg": values["foreground"],
            "cursor": values.get("cursor-color", values["foreground"]),
            "selection": values.get("selection-background", palette["8"]),
            "accent": values.get("split-divider-color", palette["4"]),
            "palette": [palette[str(i)] for i in range(16)],
            "wallpapers": len(list((wallpapers_repo / slug).iterdir())),
        })
        print(f"{slug:18} {thumb.stat().st_size // 1024} KB")
    (SITE / "themes.json").write_text(json.dumps(out, indent=1) + "\n")


if __name__ == "__main__":
    main()
