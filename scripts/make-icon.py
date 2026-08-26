#!/usr/bin/env python3
"""Regenerate the OS icon files from the brand mark.

    python3 scripts/make-icon.py        # rewrites resources/icon.png and icon.ico

The mark is a rounded slab parted by a stepped seam — the two leaves of a door
drawn apart, which is what a *limen* is. The geometry here is the same 256-unit
grid `draw_brand` in `src/limen-gui/app/brand.rs` paints from: keep the two in
step, or the window's mark and the taskbar's stop being the same drawing.

Needs Pillow (`pip install --user pillow`); nothing else in the build does, so
this is run by hand when the mark changes rather than from `build.rs`.
"""

import os

from PIL import Image, ImageChops, ImageDraw, ImageFilter

# ---- the palette, shared with the in-app mark ------------------------------ #

LIGHT = (0xF4, 0xC0, 0x78)  # bright amber
DARK = (0xF9, 0x73, 0x16)  # orange
TILE_TOP = (0x24, 0x1A, 0x10)
TILE_BOTTOM = (0x0D, 0x0A, 0x06)
SHEEN = (0xFF, 0xE0, 0xB0)

# ---- the geometry, on the 256-unit grid ------------------------------------ #

GRID = 256
TILE = 16.0  # the dark tile's inset, and its corner radius
TILE_R = 44.0
SLAB = 52.0  # the mark's own inset, and its corner radius
SLAB_R = 30.0
SEAM_JOG = 56.0  # how far the seam steps sideways...
LEAF_NEAR = (93.0, 120.6, 144.6)  # ...and where each leaf's edge starts and
LEAF_FAR = (107.0, 111.4, 135.4)  # the heights its step runs between
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


def seam_at(y, leaf):
    """A leaf's seam edge at height `y`: down, a slanted step across, down."""
    x, start, end = leaf
    if y <= start:
        return x
    if y >= end:
        return x + SEAM_JOG
    return x + SEAM_JOG * (y - start) / (end - start)


def slab_inset(y):
    """How far the slab's straight edge is pulled in by the rounding at `y`."""
    far = SLAB + SLAB_R
    d = max(far - y, y - (GRID - far), 0.0)
    return SLAB_R - max(SLAB_R**2 - d**2, 0.0) ** 0.5


def seam_rows():
    """The heights the leaves are sampled at — an even sweep plus the exact
    heights the steps turn at, so the seam's corners stay sharp."""
    steps = 256
    ys = [SLAB + (GRID - 2 * SLAB) * i / steps for i in range(steps + 1)]
    ys += [LEAF_NEAR[1], LEAF_NEAR[2], LEAF_FAR[1], LEAF_FAR[2]]
    return sorted(set(ys))


def leaf_outline(leaf, keeps_near):
    """One leaf as a closed polygon: down its outer edge, back up its seam."""
    ys = seam_rows()

    def edges(y):
        cut = seam_at(y, leaf)
        if keeps_near:
            return SLAB + slab_inset(y), cut
        return cut, GRID - SLAB - slab_inset(y)

    left = [(edges(y)[0], y) for y in ys]
    right = [(edges(y)[1], y) for y in reversed(ys)]
    return left + right


# ---- painting -------------------------------------------------------------- #


def vertical_gradient(top, bottom):
    img = Image.new("RGB", (W, W))
    draw = ImageDraw.Draw(img)
    for y in range(W):
        draw.line([(0, y), (W, y)], fill=lerp(top, bottom, y / W))
    return img


def diagonal_gradient():
    """Light at the top-left, dark at the bottom-right, across the slab — the
    same normalization `shade` uses in `draw_brand`."""
    img = Image.new("RGB", (W, W))
    pixels = img.load()
    span = 2.0 * (GRID / 2 - SLAB)
    for y in range(W):
        v = (y / SS - GRID / 2) / span + 0.5
        row = [lerp(LIGHT, DARK, v + (x / SS - GRID / 2) / span) for x in range(W)]
        for x, color in enumerate(row):
            pixels[x, y] = color
    return img


def render():
    canvas = Image.new("RGBA", (W, W), (0, 0, 0, 0))

    # The dark rounded tile the mark sits on.
    tile = Image.new("L", (W, W), 0)
    ImageDraw.Draw(tile).rounded_rectangle(
        [px(TILE), px(TILE), px(GRID - TILE), px(GRID - TILE)],
        radius=px(TILE_R),
        fill=255,
    )
    canvas.paste(vertical_gradient(TILE_TOP, TILE_BOTTOM), (0, 0), tile)

    # Both leaves at once, so the gradient runs across the pair as one slab.
    mask = Image.new("L", (W, W), 0)
    draw = ImageDraw.Draw(mask)
    for leaf, keeps_near in [(LEAF_NEAR, True), (LEAF_FAR, False)]:
        draw.polygon([(px(x), px(y)) for x, y in leaf_outline(leaf, keeps_near)], fill=255)
    canvas.paste(diagonal_gradient(), (0, 0), mask)

    # The sheen just inside every edge: the mask minus the mask eroded by the
    # stroke width, so the two leaves' facing edges each get their own gleam.
    kernel = max(3, int(px(SHEEN_W)) | 1)
    edge = ImageChops.subtract(mask, mask.filter(ImageFilter.MinFilter(kernel)))
    canvas.paste(Image.new("RGB", (W, W), SHEEN), (0, 0), edge)
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
