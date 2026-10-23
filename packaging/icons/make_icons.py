#!/usr/bin/env python3
"""Builds the application icons of Project Hoopeo from the logos in branding/.

  python3 packaging/icons/make_icons.py

For each product it writes PNG sizes, a macOS .icns and a Windows .ico into
packaging/icons/<product>/. The logo is centred on a 1024 canvas with the margin
macOS uses for app icons, on a transparent background.
"""
from pathlib import Path
from PIL import Image

ROOT = Path(__file__).resolve().parents[2]
PRODUCTS = {
    "hoopeo-studio": "hoopeo-studio.png",
    "dep-cli": "dep-cli.png",
    "dep-engine": "dep-engine.png",
    "project-hoopeo": "hoopeo-mascot.png",
}


def square(logo: Image.Image, size: int = 1024, content: int = 860) -> Image.Image:
    logo = logo.convert("RGBA")
    logo = logo.crop(logo.getbbox())
    scale = content / max(logo.size)
    logo = logo.resize((round(logo.width * scale), round(logo.height * scale)), Image.LANCZOS)
    canvas = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    canvas.paste(logo, ((size - logo.width) // 2, (size - logo.height) // 2), logo)
    return canvas


def main() -> None:
    for name, src in PRODUCTS.items():
        out = ROOT / "packaging" / "icons" / name
        out.mkdir(parents=True, exist_ok=True)
        icon = square(Image.open(ROOT / "branding" / src))
        icon.save(out / "icon.png")
        for s in (16, 32, 64, 128, 256, 512):
            icon.resize((s, s), Image.LANCZOS).save(out / f"{s}x{s}.png")
        icon.resize((256, 256), Image.LANCZOS).save(out / "128x128@2x.png")
        icon.save(out / "icon.icns", sizes=[(16, 16), (32, 32), (64, 64), (128, 128), (256, 256), (512, 512), (1024, 1024)])
        icon.save(out / "icon.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
        print("icons:", out.relative_to(ROOT))


if __name__ == "__main__":
    main()
