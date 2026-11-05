#!/usr/bin/env python3
"""Render M6 Canvas recordings with a substitute monospace font (Pillow required).

LMP_SCREEN_PREVIEW_DIR=/tmp/lmp-screens cargo test --test milestone6 screen_preview_artifacts
python3 tools/render_screens.py /tmp/lmp-screens

This checks widget geometry/clipping, not Horizon's font or GPU integration.
"""
import json
import pathlib
import sys
from PIL import Image, ImageDraw, ImageFont


def render(path):
    data = json.loads(path.read_text())
    width, height = map(int, data["size"])
    image = Image.new("RGBA", (width, height), "#111111")
    try:
        font = ImageFont.truetype("NotoSansMono-Regular.ttf", 10)
    except OSError:
        try:
            font = ImageFont.truetype("DejaVuSansMono.ttf", 10)
        except OSError:
            font = ImageFont.load_default()
    clips = [(0, 0, width, height)]
    for op in data["ops"]:
        if op[0] == "pop":
            clips.pop()
            continue
        if op[0] == "clip":
            x, y, w, h = op[1]
            a, b, c, d = clips[-1]
            clips.append((max(a, x), max(b, y), min(c, x + w), min(d, y + h)))
            continue
        color = op[-1]
        rgba = ((color >> 16) & 255, (color >> 8) & 255, color & 255, color >> 24)
        layer = Image.new("RGBA", image.size)
        draw = ImageDraw.Draw(layer)
        if op[0] == "fill":
            x, y, w, h = op[1]
            if w <= 0 or h <= 0:
                continue
            draw.rectangle((x, y, x + w - 1, y + h - 1), fill=rgba)
        else:
            draw.text(tuple(op[2]), op[1], font=font, fill=rgba, anchor="lt")
        a, b, c, d = map(int, clips[-1])
        if c > a and d > b:
            image.alpha_composite(layer.crop((a, b, c, d)), (a, b))
    image.resize((width * 2, height * 2), Image.Resampling.NEAREST).save(path.with_suffix(".png"))


if __name__ == "__main__":
    for path in pathlib.Path(sys.argv[1]).glob("*.json"):
        render(path)
