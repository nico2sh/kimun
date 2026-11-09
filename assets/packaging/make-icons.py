#!/usr/bin/env python3
"""Regenerate the desktop app's icon files from assets/logo.png.

Linux: hicolor PNGs (16-512) under linux/icons. Windows: kimun.ico. macOS:
kimun.icns. Needs Pillow (`pip install pillow`). Run from anywhere:

    python3 assets/packaging/make-icons.py
"""
from pathlib import Path

from PIL import Image

HERE = Path(__file__).resolve().parent
LOGO = HERE.parent / "logo.png"

LINUX_SIZES = [16, 24, 32, 48, 64, 128, 256, 512]
ICO_SIZES = [16, 24, 32, 48, 64, 128, 256]
ICNS_SIZES = [16, 32, 64, 128, 256, 512, 1024]


def scaled(logo: Image.Image, n: int) -> Image.Image:
    return logo.resize((n, n), Image.LANCZOS)


def main() -> None:
    logo = Image.open(LOGO).convert("RGBA")
    assert logo.size[0] == logo.size[1], "the logo must be square"

    for n in LINUX_SIZES:
        out = HERE / "linux" / "icons" / "hicolor" / f"{n}x{n}" / "apps" / "kimun.png"
        out.parent.mkdir(parents=True, exist_ok=True)
        scaled(logo, n).save(out, optimize=True)

    (HERE / "windows").mkdir(exist_ok=True)
    scaled(logo, max(ICO_SIZES)).save(
        HERE / "windows" / "kimun.ico", sizes=[(n, n) for n in ICO_SIZES]
    )

    (HERE / "macos").mkdir(exist_ok=True)
    scaled(logo, max(ICNS_SIZES)).save(
        HERE / "macos" / "kimun.icns", sizes=[(n, n) for n in ICNS_SIZES]
    )


if __name__ == "__main__":
    main()
