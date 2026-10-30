"""Draw the source app icon and the menu-bar tray icon.

Run `python3 scripts/make-icons.py && npx tauri icon src-tauri/icons/app-icon.png`
to regenerate every bundle icon size.
"""
from pathlib import Path

from PIL import Image, ImageDraw

OUT = Path(__file__).resolve().parent.parent / "src-tauri" / "icons"


def rows(draw, size, color, dot_colors, scale):
    """Three buddy-list rows: a status dot and a bar."""
    left, top = 0.22 * size, 0.27 * size
    gap, dot, bar_h = 0.19 * size, 0.11 * size, 0.07 * size
    widths = (0.42, 0.34, 0.38)
    for i, (w, dot_color) in enumerate(zip(widths, dot_colors)):
        y = top + i * gap
        draw.ellipse([left, y, left + dot, y + dot], fill=dot_color)
        bx = left + dot + 0.06 * size
        by = y + (dot - bar_h) / 2
        draw.rounded_rectangle([bx, by, bx + w * size, by + bar_h], radius=bar_h / 2 * scale, fill=color)


def app_icon(size=1024):
    img = Image.new("RGBA", (size, size), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    pad = size * 0.09
    d.rounded_rectangle([pad, pad, size - pad, size - pad], radius=size * 0.2, fill=(217, 119, 87, 255))
    white = (255, 250, 245, 255)
    rows(d, size, white, [(120, 220, 140, 255), (255, 214, 102, 255), white], 1)
    img.save(OUT / "app-icon.png")


def tray_icon(size=44):
    # macOS treats this as a template image: only the alpha channel matters.
    img = Image.new("RGBA", (size * 4, size * 4), (0, 0, 0, 0))
    d = ImageDraw.Draw(img)
    black = (0, 0, 0, 255)
    rows(d, size * 4, black, [black] * 3, 1)
    img.resize((size, size), Image.LANCZOS).save(OUT / "tray.png")


if __name__ == "__main__":
    OUT.mkdir(parents=True, exist_ok=True)
    app_icon()
    tray_icon()
