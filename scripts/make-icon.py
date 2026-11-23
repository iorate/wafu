#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.9"
# dependencies = []
# ///
"""Draws the wafu icon (a yukimi shoji: two paper panels over a pane of
glass) as wafu.ico, wafu.icns, and wafu.png.

Run from anywhere; the files go to crates/wafu/assets unless a directory is
given:

    uv run scripts/make-icon.py [DIR]

The compressed bytes of a PNG depend on the zlib that the interpreter was
built with, so a run may rewrite the files without changing a pixel.
"""

import argparse
import struct
import zlib
from dataclasses import dataclass
from pathlib import Path

SIZES = (16, 20, 24, 28, 32, 48, 64, 128, 256)
# Each icns entry is a PNG of the given size; the @2x entries, which a Retina
# display asks for, have the size of their pixel count.
ICNS_TYPES = (
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic07", 128),
    (b"ic13", 256),
    (b"ic08", 256),
    (b"ic14", 512),
    (b"ic09", 512),
    (b"ic10", 1024),
)
# Sizes drawn on the pixel grid instead of scaled down, as (margin, frame,
# stile, bar, glass line, paper height, glass height) in pixels. The design
# takes one pixel less than the size, so that the stile has a middle pixel;
# the last column and row are transparent. Scaling the 64-unit design down
# would make the lattice thinner than a pixel, and it would fade or vanish. The
# image stays even-sized so that the OS does not rescale it, while a 2-pixel
# stile would look too thick.
GRID = {
    16: (1, 1, 1, 1, 1, 2, 2),
    20: (1, 1, 1, 1, 1, 3, 3),
    24: (2, 1, 1, 1, 1, 3, 5),
    28: (2, 1, 1, 1, 1, 4, 6),
    32: (3, 2, 1, 1, 2, 4, 5),
}
# Sizes up to this get no reflections.
SIMPLE_MAX = 20
# Sizes up to this are stored uncompressed, as the icon loader of Windows has
# always read them; larger ones are PNG to keep the file small.
DIB_MAX = 48
SUPERSAMPLE = 4
# Colors and coordinates of the design, on a 64-unit canvas.
OUTER = (0xE8, 0xD9, 0xC0)
FRAME = (0x8B, 0x5A, 0x2B)
PAPER = (0xFB, 0xF8, 0xF1)
GLASS = (0xB9, 0xD3, 0xEC)
WHITE = (0xFF, 0xFF, 0xFF)
REFLECTION_ALPHA = 0.6

OUTER_RADIUS = 14
FRAME_LEFT, FRAME_TOP, FRAME_RIGHT, FRAME_BOTTOM = 7, 7, 57, 57
FRAME_RADIUS, FRAME_WIDTH = 3, 3
LATTICE_WIDTH = 1.8
GLASS_TOP, GLASS_LINE_WIDTH = 41, 2.4
# Each reflection is a parallelogram given by its bottom and top x ranges.
REFLECTIONS = (((36, 42), (50, 56)), ((30, 32.5), (44, 46.5)))


@dataclass
class Layout:
    """The design in the pixels of one canvas. Bands are [start, end)."""

    size: int
    outer_radius: float
    frame_left: float
    frame_top: float
    frame_right: float
    frame_bottom: float
    frame_width: float
    frame_radius: float
    glass: tuple
    lattice: tuple
    bars: list
    reflections: list
    reflection_top: float
    reflection_bottom: float


def scaled_layout(size):
    k = size / 64
    half = FRAME_WIDTH / 2

    def band(center, width):
        return ((center - width / 2) * k, (center + width / 2) * k)

    bars = [FRAME_TOP + (GLASS_TOP - FRAME_TOP) * i / 3 for i in (1, 2)]
    return Layout(
        size=size,
        outer_radius=OUTER_RADIUS * k,
        frame_left=(FRAME_LEFT - half) * k,
        frame_top=(FRAME_TOP - half) * k,
        frame_right=(FRAME_RIGHT + half) * k,
        frame_bottom=(FRAME_BOTTOM + half) * k,
        frame_width=FRAME_WIDTH * k,
        frame_radius=(FRAME_RADIUS + half) * k,
        glass=band(GLASS_TOP, GLASS_LINE_WIDTH),
        lattice=band(32, LATTICE_WIDTH),
        bars=[band(bar, LATTICE_WIDTH) for bar in bars],
        reflections=[
            [[x * k for x in bottom], [x * k for x in top]]
            for bottom, top in REFLECTIONS
        ],
        reflection_top=GLASS_TOP * k,
        reflection_bottom=FRAME_BOTTOM * k,
    )


def hinted_layout(size):
    """Lays the design on whole pixels from the table, so that the paper
    panels are equal."""
    k = size / 64
    margin, frame, stile, bar, glass_line, paper, glass_height = GRID[size]
    drawn = size - 1
    inner_top = margin + frame
    inner_bottom = drawn - inner_top
    stile_left = inner_top + (inner_bottom - inner_top - stile) // 2
    bars = []
    y = inner_top
    for _ in range(2):
        y += paper
        bars.append((y, y + bar))
        y += bar
    y += paper
    glass = (y, y + glass_line)
    assert glass[1] + glass_height == inner_bottom, size
    assert (inner_bottom - inner_top - stile) % 2 == 0, size
    reflections = (
        []
        if size <= SIMPLE_MAX
        else [
            [[x * k for x in bottom], [x * k for x in top]]
            for bottom, top in REFLECTIONS
        ]
    )
    return Layout(
        size=drawn,
        outer_radius=OUTER_RADIUS * k,
        frame_left=margin,
        frame_top=margin,
        frame_right=drawn - margin,
        frame_bottom=drawn - margin,
        frame_width=frame,
        frame_radius=(FRAME_RADIUS + FRAME_WIDTH / 2) * k,
        glass=glass,
        lattice=(stile_left, stile_left + stile),
        bars=bars,
        reflections=reflections,
        reflection_top=glass[1],
        reflection_bottom=inner_bottom,
    )


def layout_for(size):
    return hinted_layout(size) if size in GRID else scaled_layout(size)


def inside_rounded_rect(x, y, left, top, right, bottom, radius):
    if x < left or x >= right or y < top or y >= bottom:
        return False
    cx = min(max(x, left + radius), right - radius)
    cy = min(max(y, top + radius), bottom - radius)
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius**2


def inside_reflection(x, y, layout):
    if y < layout.reflection_top or y >= layout.reflection_bottom:
        return False
    t = (layout.reflection_bottom - y) / (
        layout.reflection_bottom - layout.reflection_top
    )
    for (bottom_left, bottom_right), (top_left, top_right) in layout.reflections:
        left = bottom_left + (top_left - bottom_left) * t
        right = bottom_right + (top_right - bottom_right) * t
        if left <= x < right:
            return True
    return False


def inside_band(value, band):
    return band[0] <= value < band[1]


def blend(under, over, alpha):
    return tuple(round(u * (1 - alpha) + o * alpha) for u, o in zip(under, over))


def design(x, y, layout):
    """The RGBA of a point of the canvas."""
    if not inside_rounded_rect(
        x, y, 0, 0, layout.size, layout.size, layout.outer_radius
    ):
        return (0, 0, 0, 0)
    if not inside_rounded_rect(
        x,
        y,
        layout.frame_left,
        layout.frame_top,
        layout.frame_right,
        layout.frame_bottom,
        layout.frame_radius,
    ):
        return (*OUTER, 255)
    width = layout.frame_width
    if not inside_rounded_rect(
        x,
        y,
        layout.frame_left + width,
        layout.frame_top + width,
        layout.frame_right - width,
        layout.frame_bottom - width,
        max(layout.frame_radius - width, 0),
    ):
        return (*FRAME, 255)
    if inside_band(y, layout.glass):
        return (*FRAME, 255)
    if y >= layout.glass[1]:
        if inside_reflection(x, y, layout):
            return (*blend(GLASS, WHITE, REFLECTION_ALPHA), 255)
        return (*GLASS, 255)
    if inside_band(x, layout.lattice):
        return (*FRAME, 255)
    for bar in layout.bars:
        if inside_band(y, bar):
            return (*FRAME, 255)
    return (*PAPER, 255)


def render(size):
    layout = layout_for(size)
    rows = []
    for py in range(layout.size):
        row = bytearray()
        for px in range(layout.size):
            r = g = b = a = 0
            for sy in range(SUPERSAMPLE):
                for sx in range(SUPERSAMPLE):
                    sr, sg, sb, sa = design(
                        px + (sx + 0.5) / SUPERSAMPLE,
                        py + (sy + 0.5) / SUPERSAMPLE,
                        layout,
                    )
                    r += sr * sa
                    g += sg * sa
                    b += sb * sa
                    a += sa
            count = SUPERSAMPLE * SUPERSAMPLE
            if a:
                row += bytes((r // a, g // a, b // a, a // count))
            else:
                row += b"\0\0\0\0"
        row += b"\0\0\0\0" * (size - layout.size)
        rows.append(bytes(row))
    rows += [b"\0\0\0\0" * size] * (size - layout.size)
    return rows


def png(size, rows):
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    raw = b"".join(b"\0" + row for row in rows)
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", size, size, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )


def dib(size, rows):
    """A 32-bit bottom-up DIB with the AND mask that an icon carries."""
    mask_stride = (size + 31) // 32 * 4
    pixels = b""
    mask = b""
    for row in reversed(rows):
        line = bytearray()
        bits = 0
        for i in range(size):
            r, g, b, a = row[4 * i : 4 * i + 4]
            line += bytes((b, g, r, a))
            bits = bits << 1 | (a == 0)
        pixels += line
        mask += (bits << (mask_stride * 8 - size)).to_bytes(mask_stride, "big")
    header = struct.pack(
        "<IiiHHIIiiII",
        40,
        size,
        size * 2,
        1,
        32,
        0,
        len(pixels) + len(mask),
        0,
        0,
        0,
        0,
    )
    return header + pixels + mask


def ico(images):
    header = struct.pack("<HHH", 0, 1, len(images))
    offset = len(header) + 16 * len(images)
    entries = b""
    data = b""
    for size, blob in images:
        entries += struct.pack(
            "<BBBBHHII",
            size % 256,
            size % 256,
            0,
            0,
            1,
            32,
            len(blob),
            offset + len(data),
        )
        data += blob
    return header + entries + data


def icns(pngs):
    entries = b"".join(
        kind + struct.pack(">I", 8 + len(blob)) + blob for kind, blob in pngs
    )
    return b"icns" + struct.pack(">I", 8 + len(entries)) + entries


def main():
    assets = Path(__file__).resolve().parent.parent / "crates" / "wafu" / "assets"
    parser = argparse.ArgumentParser(description="Draws the wafu icon.")
    parser.add_argument("dir", nargs="?", type=Path, default=assets)
    out = parser.parse_args().dir
    out.mkdir(parents=True, exist_ok=True)
    rendered = {
        size: render(size) for size in sorted({*SIZES, *(s for _, s in ICNS_TYPES)})
    }
    images = [
        (
            size,
            dib(size, rendered[size]) if size <= DIB_MAX else png(size, rendered[size]),
        )
        for size in SIZES
    ]
    (out / "wafu.ico").write_bytes(ico(images))
    (out / "wafu.icns").write_bytes(
        icns([(kind, png(size, rendered[size])) for kind, size in ICNS_TYPES])
    )
    (out / "wafu.png").write_bytes(png(SIZES[-1], rendered[SIZES[-1]]))


if __name__ == "__main__":
    main()
