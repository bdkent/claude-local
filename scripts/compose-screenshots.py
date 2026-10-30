"""Lay out UI screenshots side by side for the README.

Usage: python3 scripts/compose-screenshots.py OUT.png BORDER_HEX IN1.png IN2.png ...
"""
import sys

from PIL import Image, ImageDraw

out, border, *paths = sys.argv[1:]
shots = [Image.open(p).convert("RGBA") for p in paths]
gap, radius, pad = 48, 22, 8
w = sum(s.width for s in shots) + gap * (len(shots) - 1) + pad * 2
h = max(s.height for s in shots) + pad * 2
canvas = Image.new("RGBA", (w, h), (0, 0, 0, 0))

x = pad
for shot in shots:
    mask = Image.new("L", shot.size, 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, shot.width - 1, shot.height - 1], radius, fill=255)
    canvas.paste(shot, (x, pad), mask)
    ImageDraw.Draw(canvas).rounded_rectangle(
        [x, pad, x + shot.width - 1, pad + shot.height - 1], radius, outline=border, width=2
    )
    x += shot.width + gap

canvas.save(out, optimize=True)
