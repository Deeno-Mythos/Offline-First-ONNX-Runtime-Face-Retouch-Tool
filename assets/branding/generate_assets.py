"""Rebuild the hand-authored vector brand and its lossless raster derivatives.

Requires Pillow. No retouching, color grading, AI generation, or model training.
The chosen splash photograph is embedded directly from assets/demo-portrait.png.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

HERE = Path(__file__).resolve().parent
CROWN = [
    (42, 131), (152, 311), (144, 202), (213, 325), (256, 36),
    (299, 325), (368, 202), (360, 311), (470, 131), (384, 420), (128, 420),
]
BASE = [(128, 438), (384, 438), (392, 462), (120, 462)]
GOLD = "#F2C526"
INK = "#17191C"
IVORY = "#F4F2E9"


def svg(crown):
    crown_path = "M " + " L ".join(f"{x} {y}" for x, y in CROWN) + " Z"
    base_path = "M " + " L ".join(f"{x} {y}" for x, y in BASE) + " Z"
    return f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512" role="img" aria-labelledby="title desc">
  <title id="title">Hastur Retouch crown</title>
  <desc id="desc">A symmetrical five-spire crown with a tall central point and a detached golden base.</desc>
  <path fill="{crown}" d="{crown_path}"/>
  <path fill="{GOLD}" d="{base_path}"/>
</svg>
'''


def raster(size, crown=INK, icon=False):
    # Supersampling preserves the exact same geometry at 16px and 1024px.
    scale = max(4, 2048 // size)
    side = size * scale
    im = Image.new("RGBA", (side, side), (0, 0, 0, 0))
    draw = ImageDraw.Draw(im)
    if icon:
        draw.rounded_rectangle((0, 0, side - 1, side - 1), side * 0.19, fill=INK)
    factor = side / 512
    for points, fill in [(CROWN, crown), (BASE, GOLD)]:
        draw.polygon([(x * factor, y * factor) for x, y in points], fill=fill)
    return im.resize((size, size), Image.Resampling.LANCZOS)


def font(size):
    return ImageFont.truetype("C:/Windows/Fonts/segoeui.ttf", size)


def main():
    HERE.mkdir(parents=True, exist_ok=True)
    (HERE / "hastur-mark.svg").write_text(svg(INK), encoding="utf-8")
    (HERE / "hastur-mark-light.svg").write_text(svg(INK), encoding="utf-8")
    (HERE / "hastur-mark-dark.svg").write_text(svg(IVORY), encoding="utf-8")
    raster(1024).save(HERE / "hastur-mark.png")
    raster(1024, IVORY).save(HERE / "hastur-mark-dark.png")
    icon = raster(1024, IVORY, icon=True)
    icon.save(HERE / "hastur-icon.png")
    icon.save(HERE / "hastur.ico", format="ICO", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    sheet = Image.new("RGB", (1440, 900), "#EFEDE5")
    draw = ImageDraw.Draw(sheet)
    draw.rectangle((720, 0, 1439, 899), fill="#17191C")
    sheet.paste(raster(480, INK), (120, 70), raster(480, INK))
    sheet.paste(raster(480, IVORY), (840, 70), raster(480, IVORY))
    for x, color in ((72, INK), (792, IVORY)):
        draw.text((x, 570), "HASTUR RETOUCH", fill=color, font=font(44))
        draw.text((x, 635), "Five spires. One clear identity.", fill=color, font=font(24))
        for offset, size in ((0, 16), (68, 24), (144, 32), (228, 48), (332, 64)):
            preview = raster(size, IVORY, icon=True)
            sheet.paste(preview, (x + offset, 720), preview)
            draw.text((x + offset, 802), f"{size}px", fill=color, font=font(18))
    sheet.save(HERE / "brand-preview.png")
    print("Wrote vector masters, transparent PNGs, seven-size ICO, icon PNG, and preview.")


if __name__ == "__main__":
    main()
