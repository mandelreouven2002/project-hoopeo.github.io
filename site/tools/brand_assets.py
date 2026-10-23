#!/usr/bin/env python3
"""Makes the web copies of the brand assets, into site/assets/brand/.

  python3 site/tools/brand_assets.py          (needs Pillow; the output is committed, so the
                                               site build itself does not need it)

Reads the masters in branding/ and never changes them. Writes:
  * the logos and product icons as WebP with a PNG fallback, at the sizes the pages use (1x, 2x);
  * favicon.ico, icon-192.png, icon-512.png, apple-touch-icon.png and site.webmanifest, from the
    application icon of Project Hoopeo (the mascot centred on a square, the same picture as
    packaging/icons/project-hoopeo in the private repository);
  * social images: og-default.png (1200 x 630) and one per main section.
"""
import json
import sys
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parents[2]
SRC = ROOT / "branding"
OUT = ROOT / "site" / "assets" / "brand"
FONTS = ROOT / "site" / "assets" / "fonts"

# The brand colours, sampled from the masters (see site/assets/css/tokens.css)
BG_LIGHT = (244, 247, 251)
NAVY = (1, 26, 62)
BLUE = (3, 112, 222)
ORANGE = (252, 145, 10)
TEAL = (2, 208, 209)
MUTED = (71, 90, 118)

# name, master, [widths]. Widths are the sizes at 1x and 2x.
SETS = [
    ("wordmark", "project-hoopeo-wordmark.png", [520, 1040]),
    ("wordmark-dark", "project-hoopeo-wordmark-dark.png", [520, 1040]),
    ("mascot", "hoopeo-mascot.png", [40, 80, 240, 480]),
    ("dep", "dep.png", [96, 192]),
    ("dep-engine", "dep-engine.png", [64, 128]),
    ("dep-cli", "dep-cli.png", [64, 128]),
    ("hoopeo-studio", "hoopeo-studio.png", [64, 128]),
    ("dep-js", "dep-js.png", [64, 128]),
]


def resized(im: Image.Image, width: int) -> Image.Image:
    height = round(im.height * width / im.width)
    return im.resize((width, height), Image.LANCZOS)


def save_pair(im: Image.Image, stem: str) -> None:
    """WebP, and a PNG fallback quantised to a palette so it stays small."""
    im.save(OUT / f"{stem}.webp", "WEBP", quality=88, method=6)
    q = im.quantize(colors=256, method=Image.Quantize.FASTOCTREE, dither=Image.Dither.NONE)
    q.save(OUT / f"{stem}.png", optimize=True)


def square(logo: Image.Image, size: int = 1024, content: int = 860) -> Image.Image:
    logo = logo.convert("RGBA")
    logo = logo.crop(logo.getbbox())
    scale = content / max(logo.size)
    logo = logo.resize((round(logo.width * scale), round(logo.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(logo, ((size - logo.width) // 2, (size - logo.height) // 2), logo)
    return canvas


def font(name: str, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(str(FONTS / f"ibm-plex-sans-{name}-normal.woff2"), size)


def og(title: str, subtitle: str, stem: str, wordmark: Image.Image) -> None:
    """1200 x 630: the wordmark on the light background; a section image adds its title."""
    img = Image.new("RGB", (1200, 630), BG_LIGHT)
    d = ImageDraw.Draw(img)
    if title:
        wm = resized(wordmark, 420)
        img.paste(wm, (70, 60), wm)
        d.rectangle((70, 232, 190, 238), fill=ORANGE)
        d.text((70, 280), title, font=font("latin-600", 84), fill=NAVY)
        d.text((70, 392), subtitle, font=font("latin-400", 36), fill=MUTED)
        d.rectangle((0, 618, 1200, 630), fill=BLUE)
    else:
        wm = resized(wordmark, 960)
        img.paste(wm, ((1200 - wm.width) // 2, (630 - wm.height) // 2), wm)
        d.rectangle((0, 618, 1200, 630), fill=BLUE)
    img.save(OUT / f"{stem}.png", optimize=True)


def main() -> int:
    OUT.mkdir(parents=True, exist_ok=True)
    for name, master, widths in SETS:
        im = Image.open(SRC / master).convert("RGBA")
        for w in widths:
            save_pair(resized(im, w), f"{name}-{w}")

    # favicons and touch icons, from the application icon
    icon = square(Image.open(SRC / "hoopeo-mascot.png"))
    icon.resize((192, 192), Image.LANCZOS).save(OUT / "icon-192.png", optimize=True)
    icon.resize((512, 512), Image.LANCZOS).save(OUT / "icon-512.png", optimize=True)
    plate = Image.new("RGBA", (180, 180), (255, 255, 255, 255))   # iOS draws no transparency
    small = icon.resize((150, 150), Image.LANCZOS)
    plate.paste(small, (15, 15), small)
    plate.convert("RGB").save(OUT / "apple-touch-icon.png", optimize=True)
    icon.save(OUT / "favicon.ico", sizes=[(16, 16), (32, 32), (48, 48)])
    (OUT / "site.webmanifest").write_text(json.dumps({
        "name": "Project Hoopeo",
        "short_name": "Hoopeo",
        "description": "Database-Enforced Privacy",
        "icons": [
            {"src": "icon-192.png", "sizes": "192x192", "type": "image/png"},
            {"src": "icon-512.png", "sizes": "512x512", "type": "image/png"},
        ],
        "theme_color": "#011A3E",
        "background_color": "#F4F7FB",
        "display": "browser",
    }, indent=2) + "\n")

    # social images
    wm = Image.open(SRC / "project-hoopeo-wordmark.png").convert("RGBA")
    og("", "", "og-default", wm)
    og("Privacy as Code", "How DEP puts privacy rules in the database", "og-privacy-as-code", wm)
    og("Docs", "Guides and reference for DEP SQL and DEP Engine", "og-docs", wm)
    og("Playground", "DEP in your browser, with nothing to install", "og-playground", wm)
    og("Install", "DEP Engine, DEP CLI, Hoopeo Studio and DEP.js", "og-install", wm)

    for f in sorted(OUT.iterdir()):
        print(f"{f.stat().st_size // 1024:5d} KB  {f.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
