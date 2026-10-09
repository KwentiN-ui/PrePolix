#!/usr/bin/env python3
"""Draws the prepolix program icon: a triangle split into four elements, coloured like a
contour plot, on a light rounded square. Writes crates/plx-app/assets/icon/prepolix.png
(256 px, the window icon) and prepolix.ico (16 to 256 px, for the Windows installer).

    python3 scripts/make_icon.py

Needs Pillow; only for changing the icon, the outputs are checked in.
"""

import pathlib

from PIL import Image, ImageDraw

SCALE = 4  # drawn larger, then scaled down for smooth edges
SIZE = 256 * SCALE


def point(x, y):
    return (x * SIZE, y * SIZE)


def main():
    image = Image.new("RGBA", (SIZE, SIZE), (0, 0, 0, 0))
    draw = ImageDraw.Draw(image)
    draw.rounded_rectangle(
        [point(0.03, 0.03), point(0.97, 0.97)],
        radius=0.18 * SIZE,
        fill=(236, 240, 245, 255),
        outline=(120, 130, 145, 255),
        width=int(0.02 * SIZE),
    )
    # Corners and edge midpoints of the big triangle.
    a, b, c = (0.12, 0.84), (0.88, 0.84), (0.5, 0.16)
    mid = lambda p, q: ((p[0] + q[0]) / 2, (p[1] + q[1]) / 2)
    ab, bc, ca = mid(a, b), mid(b, c), mid(c, a)
    elements = [
        ((a, ab, ca), (20, 60, 230)),
        ((ab, b, bc), (40, 210, 230)),
        ((ab, bc, ca), (90, 220, 60)),
        ((ca, bc, c), (235, 40, 30)),
    ]
    edge = (25, 30, 40, 255)
    for corners, colour in elements:
        draw.polygon([point(*p) for p in corners], fill=colour + (255,))
    for corners, _ in elements:
        points = [point(*p) for p in corners]
        draw.line(points + [points[0]], fill=edge, width=int(0.022 * SIZE), joint="curve")

    out = pathlib.Path(__file__).resolve().parent.parent / "crates/plx-app/assets/icon"
    out.mkdir(parents=True, exist_ok=True)
    icon = image.resize((256, 256), Image.LANCZOS)
    icon.save(out / "prepolix.png", optimize=True)
    icon.save(out / "prepolix.ico", sizes=[(s, s) for s in (16, 24, 32, 48, 64, 128, 256)])
    print(f"Icon in {out}")


if __name__ == "__main__":
    main()
