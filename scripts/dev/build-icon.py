"""Renders the AgentGo mark into the application icon set.

The geometry is the `#i-mark` symbol of ui/chrome.html (viewBox 48x48): the
orange halo, the light "A" and the orange arrow. It is drawn on a graphite
tile so the icon reads on both light and dark taskbars.

Outputs (checked in, regenerate with `python scripts/dev/build-icon.py`):
  crates/agentgo-app/assets/agentgo.ico   multi-size icon, embedded into agentgo.exe
  crates/agentgo-cef/assets/icon-{32,64,128,256}.png   window/notification icons
  ui/assets/icons/agentgo-256.png         for documentation and the installer
"""
import math
import struct
from pathlib import Path

from PIL import Image, ImageDraw

ROOT = Path(__file__).resolve().parents[2]
ORANGE = (255, 106, 43, 255)
INK = (244, 239, 230, 255)
TILE_TOP = (27, 31, 39, 255)
TILE_BOTTOM = (15, 18, 23, 255)
SUPERSAMPLE = 8
ICO_SIZES = (16, 24, 32, 48, 64, 128, 256)
PNG_SIZES = (32, 64, 128, 256)


def bezier(p0, p1, p2, p3, steps=48):
    out = []
    for i in range(steps + 1):
        t = i / steps
        u = 1 - t
        out.append((
            u ** 3 * p0[0] + 3 * u * u * t * p1[0] + 3 * u * t * t * p2[0] + t ** 3 * p3[0],
            u ** 3 * p0[1] + 3 * u * u * t * p1[1] + 3 * u * t * t * p2[1] + t ** 3 * p3[1],
        ))
    return out


def stroke(draw, points, width, color, square_caps=True):
    """Draws a polyline as filled polygons so the caps can be square."""
    half = width / 2
    for index in range(len(points) - 1):
        (x0, y0), (x1, y1) = points[index], points[index + 1]
        length = math.hypot(x1 - x0, y1 - y0)
        if length == 0:
            continue
        dx, dy = (x1 - x0) / length, (y1 - y0) / length
        if square_caps:
            if index == 0:
                x0, y0 = x0 - dx * half, y0 - dy * half
            if index == len(points) - 2:
                x1, y1 = x1 + dx * half, y1 + dy * half
        nx, ny = -dy * half, dx * half
        draw.polygon([(x0 + nx, y0 + ny), (x1 + nx, y1 + ny), (x1 - nx, y1 - ny), (x0 - nx, y0 - ny)], fill=color)
    # Round the joints between segments so thin strokes stay solid.
    for x, y in points[1:-1]:
        draw.ellipse([x - half, y - half, x + half, y + half], fill=color)


def render(size):
    big = size * SUPERSAMPLE
    image = Image.new("RGBA", (big, big), (0, 0, 0, 0))

    # Graphite tile with a soft vertical gradient and rounded corners.
    tile = Image.new("RGBA", (big, big))
    pixels = tile.load()
    for y in range(big):
        mix = y / max(1, big - 1)
        row = tuple(round(TILE_TOP[c] * (1 - mix) + TILE_BOTTOM[c] * mix) for c in range(4))
        for x in range(big):
            pixels[x, y] = row
    mask = Image.new("L", (big, big), 0)
    ImageDraw.Draw(mask).rounded_rectangle([0, 0, big - 1, big - 1], radius=round(big * 0.22), fill=255)
    image.paste(tile, (0, 0), mask)

    # The mark occupies the central 70% of the tile. Small sizes get a
    # slightly heavier stroke so the shape survives downsampling.
    boost = 1.0 if size >= 48 else 1.0 + (48 - size) / 48 * 0.9
    scale = big * 0.70 / 48
    offset = big * 0.15
    point = lambda x, y: (offset + x * scale, offset + y * scale)
    layer = Image.new("RGBA", (big, big), (0, 0, 0, 0))
    draw = ImageDraw.Draw(layer)

    # Halo: ellipse stroke centred at (24, 9), rx 12, ry 3.5, stroke 2.8.
    width = 2.8 * boost * scale
    cx, cy = point(24, 9)
    rx, ry = 12 * scale, 3.5 * scale
    draw.ellipse([cx - rx - width / 2, cy - ry - width / 2, cx + rx + width / 2, cy + ry + width / 2], fill=ORANGE)
    draw.ellipse([cx - rx + width / 2, cy - ry + width / 2, cx + rx - width / 2, cy + ry - width / 2], fill=(0, 0, 0, 0))

    # "A": M12 40 22.4 14h3.2L36 40 and the crossbar M17 31h14.
    stroke(draw, [point(12, 40), point(22.4, 14), point(25.6, 14), point(36, 40)], 3.5 * boost * scale, INK)
    stroke(draw, [point(17, 31), point(31, 31)], 3.5 * boost * scale, INK)

    # Arrow: M11 37c8-3 16-9 25-19 then the head m-6 0h6v6.
    curve = [point(x, y) for x, y in bezier((11, 37), (19, 34), (27, 28), (36, 18))]
    stroke(draw, curve, 2.7 * boost * scale, ORANGE)
    stroke(draw, [point(30, 18), point(36, 18), point(36, 24)], 2.7 * boost * scale, ORANGE)

    image.alpha_composite(layer)
    return image.resize((size, size), Image.LANCZOS)


def png_bytes(image):
    import io
    buffer = io.BytesIO()
    image.save(buffer, format="PNG", optimize=True)
    return buffer.getvalue()


def write_ico(path, sizes):
    entries = [(size, png_bytes(render(size))) for size in sizes]
    header = struct.pack("<HHH", 0, 1, len(entries))
    offset = 6 + 16 * len(entries)
    directory, blobs = b"", b""
    for size, data in entries:
        directory += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset + len(blobs))
        blobs += data
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(header + directory + blobs)
    return sum(len(data) for _, data in entries)


def main():
    ico = ROOT / "crates" / "agentgo-app" / "assets" / "agentgo.ico"
    total = write_ico(ico, ICO_SIZES)
    print(f"wrote {ico.relative_to(ROOT)} ({ico.stat().st_size} bytes, {len(ICO_SIZES)} sizes, payload {total})")
    for size in PNG_SIZES:
        target = ROOT / "crates" / "agentgo-cef" / "assets" / f"icon-{size}.png"
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_bytes(png_bytes(render(size)))
        print(f"wrote {target.relative_to(ROOT)}")
    docs = ROOT / "ui" / "assets" / "icons" / "agentgo-256.png"
    docs.parent.mkdir(parents=True, exist_ok=True)
    docs.write_bytes(png_bytes(render(256)))
    # A contact sheet makes visual review of every size easy.
    sheet = Image.new("RGBA", (sum(ICO_SIZES) + 16 * (len(ICO_SIZES) + 1), 288), (40, 44, 52, 255))
    x = 16
    for size in ICO_SIZES:
        sheet.alpha_composite(render(size), (x, 16))
        x += size + 16
    sheet.save(ROOT / "target" / "icon-sheet.png")
    print("wrote target/icon-sheet.png")


if __name__ == "__main__":
    main()
