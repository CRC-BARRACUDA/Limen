#!/usr/bin/env python3
"""Regenerate the OS icon files from the brand mark.

    python3 scripts/make-icon.py        # rewrites resources/icon.png and icon.ico

The mark is two brackets with a lit gap between them. The brackets are drawn
only to make that gap visible: a *limen* is not the door, it is the threshold,
the part you cross. The geometry here is the same 256-unit grid `draw_brand` in
`src/limen-gui/app/brand.rs` paints from — and a test in that crate reads this
file to check the numbers still agree, because the window's mark and the
taskbar's have to be one drawing rather than two that resemble each other.

Needs Pillow (`pip install --user pillow`); nothing else in the build does, so
this is run by hand when the mark changes rather than from `build.rs`.
"""

import os

from PIL import Image, ImageChops, ImageDraw, ImageFilter

# ---- the palette, shared with the in-app mark ------------------------------ #

LIGHT = (0xF4, 0xC0, 0x78)  # bright amber
DARK = (0xF9, 0x73, 0x16)  # orange
SHEEN = (0xFF, 0xE0, 0xB0)

# ---- the geometry, on the 256-unit grid ------------------------------------ #

GRID = 256
THICK = 27.0  # how heavy a bracket's stroke is...
OUT_NEAR = 32.0  # ...and the pair's outer edges, which are also each
OUT_FAR = 224.0  # bracket's top and bottom
ARM_NEAR = 105.0  # where the arms stop on each side — everything
ARM_FAR = 151.0  # between the two is threshold
MARK_R = 4.0  # the corner radius on the marks standing in the gap
SHEEN_W = 1.4

SS = 4  # supersampling: everything is drawn at 4x and filtered down
W = GRID * SS
SIZES = [16, 32, 48, 64, 128, 256]


def px(v):
    """A grid coordinate in the supersampled canvas."""
    return v * SS


def lerp(a, b, t):
    t = max(0.0, min(1.0, t))
    return tuple(round(x + (y - x) * t) for x, y in zip(a, b))


def bracket_rects(near):
    """One bracket as three rectangles — spine, top arm, bottom arm — that share
    edges but never overlap. Drawn overlapping, the mark would look identical at
    full opacity and wrong the moment it fades."""
    if near:
        spine = OUT_NEAR + THICK
        return [
            (OUT_NEAR, OUT_NEAR, spine, OUT_FAR),
            (spine, OUT_NEAR, ARM_NEAR, OUT_NEAR + THICK),
            (spine, OUT_FAR - THICK, ARM_NEAR, OUT_FAR),
        ]
    spine = OUT_FAR - THICK
    return [
        (spine, OUT_NEAR, OUT_FAR, OUT_FAR),
        (ARM_FAR, OUT_NEAR, spine, OUT_NEAR + THICK),
        (ARM_FAR, OUT_FAR - THICK, spine, OUT_FAR),
    ]


def bracket_outline(near):
    """The same bracket as one closed outline, clockwise — the sheen has to run
    round the whole letter, not round each piece it is built from."""
    if near:
        spine = OUT_NEAR + THICK
        return [
            (OUT_NEAR, OUT_NEAR),
            (ARM_NEAR, OUT_NEAR),
            (ARM_NEAR, OUT_NEAR + THICK),
            (spine, OUT_NEAR + THICK),
            (spine, OUT_FAR - THICK),
            (ARM_NEAR, OUT_FAR - THICK),
            (ARM_NEAR, OUT_FAR),
            (OUT_NEAR, OUT_FAR),
        ]
    spine = OUT_FAR - THICK
    return [
        (OUT_FAR, OUT_NEAR),
        (ARM_FAR, OUT_NEAR),
        (ARM_FAR, OUT_NEAR + THICK),
        (spine, OUT_NEAR + THICK),
        (spine, OUT_FAR - THICK),
        (ARM_FAR, OUT_FAR - THICK),
        (ARM_FAR, OUT_FAR),
        (OUT_FAR, OUT_FAR),
    ]


def gap_marks():
    """A tick level with each pair of arms, and the crossing between them: the
    old mark's seam, continued across the opening as a broken line."""
    return [
        (120.0, 46.0, 136.0, 54.0),
        (124.0, 111.0, 132.0, 145.0),
        (120.0, 202.0, 136.0, 210.0),
    ]


# ---- painting -------------------------------------------------------------- #


def diagonal_gradient():
    """Light at the top-left, dark at the bottom-right, across the slab — the
    same normalization `shade` uses in `draw_brand`."""
    img = Image.new("RGB", (W, W))
    pixels = img.load()
    span = 2.0 * (GRID / 2 - OUT_NEAR)
    for y in range(W):
        v = (y / SS - GRID / 2) / span + 0.5
        row = [lerp(LIGHT, DARK, v + (x / SS - GRID / 2) / span) for x in range(W)]
        for x, color in enumerate(row):
            pixels[x, y] = color
    return img


def render():
    canvas = Image.new("RGBA", (W, W), (0, 0, 0, 0))

    # No tile: the icon is the mark on transparency, so it sits on whatever the
    # taskbar, the dock or Explorer puts behind it rather than carrying its own
    # dark square everywhere. The in-app mark still paints one (`show_tile`).

    # Both brackets at once, so the gradient runs across the pair as one object.
    mask = Image.new("L", (W, W), 0)
    draw = ImageDraw.Draw(mask)
    for near in (True, False):
        draw.polygon([(px(x), px(y)) for x, y in bracket_outline(near)], fill=255)
    canvas.paste(diagonal_gradient(), (0, 0), mask)

    # The sheen just inside every edge: the mask minus the mask eroded by the
    # stroke width, so each bracket's facing edge gets its own gleam.
    kernel = max(3, int(px(SHEEN_W)) | 1)
    edge = ImageChops.subtract(mask, mask.filter(ImageFilter.MinFilter(kernel)))
    canvas.paste(Image.new("RGB", (W, W), SHEEN), (0, 0), edge)

    # What stands in the gap. Flat colour, not the gradient — these are the
    # brightest things in the mark and should not dim as they go down it.
    marks = Image.new("L", (W, W), 0)
    mdraw = ImageDraw.Draw(marks)
    crossing = Image.new("L", (W, W), 0)
    cdraw = ImageDraw.Draw(crossing)
    for i, (x0, y0, x1, y1) in enumerate(gap_marks()):
        box = [px(x0), px(y0), px(x1), px(y1)]
        (cdraw if i == 1 else mdraw).rounded_rectangle(box, radius=px(MARK_R), fill=255)
    canvas.paste(Image.new("RGB", (W, W), LIGHT), (0, 0), marks)
    canvas.paste(Image.new("RGB", (W, W), DARK), (0, 0), crossing)
    return canvas


def main():
    root = os.path.join(os.path.dirname(os.path.abspath(__file__)), os.pardir)
    out = os.path.join(root, "resources")
    art = render()
    # Every size is filtered down from the 4x canvas rather than from the one
    # above it, so 16px is as clean as the mark allows.
    scaled = [art.resize((n, n), Image.LANCZOS) for n in SIZES]
    scaled[-1].save(os.path.join(out, "icon.png"))
    # `append_images` rather than `sizes=`: the latter would have Pillow shrink
    # the 256px frame for each entry, which is a filter pass worse than taking
    # every frame off the 4x canvas.
    scaled[-1].save(
        os.path.join(out, "icon.ico"),
        sizes=[(n, n) for n in SIZES],
        append_images=scaled[:-1],
    )
    print("wrote resources/icon.png and resources/icon.ico", SIZES)


if __name__ == "__main__":
    main()
