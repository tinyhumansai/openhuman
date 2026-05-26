from __future__ import annotations

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont


def generate_thumbnail(text: str, out_path: str, font_path: str, color: str):
    img = Image.new("RGB", (1080, 1920), color="#111111")
    draw = ImageDraw.Draw(img)
    font = ImageFont.truetype(font_path, 96)
    draw.rounded_rectangle((60, 640, 1020, 1280), radius=35, fill=color)
    draw.text((100, 760), text[:70], fill="white", font=font)
    Path(out_path).parent.mkdir(parents=True, exist_ok=True)
    img.save(out_path, "PNG")
