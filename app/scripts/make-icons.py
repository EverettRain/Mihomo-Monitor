#!/usr/bin/env python3
"""Builds the app icon masters from the artwork, the way Claude-Hydra's icons are made.

    python3 app/scripts/make-icons.py          (needs numpy, scipy and Pillow)
    cd app
    pnpm tauri icon src-tauri/icons-src/icon.png -o src-tauri/icons
    pnpm tauri icon src-tauri/icons-src/icon-macos.png -o <temp dir>   # then copy its icon.icns
    rm -rf src-tauri/icons/android src-tauri/icons/ios src-tauri/icons/*Logo.png

The artwork (`icons-src/source.png`) is drawn on white. Its background becomes a sky-blue
gradient: the white reached from the edges is filled, with the dark outlines thickened while
filling so gaps in them do not let the fill into white parts of the drawing (the headband, the
apron); afterwards the fill grows back up to the lines, and line edges are unmixed from white so
no pale halo remains. Then:

- `icons-src/icon.png`: 1024 px, full bleed in a continuous-corner square (squircle). Source of
  the Windows icon, the PNG sizes and the window / tray icon.
- `icons-src/icon-macos.png`: 1024 px on Apple's icon template, the squircle 824 px wide with a
  100 px margin and a soft shadow, as macOS expects in the Dock and Finder. Source of icon.icns.
- `src/assets/app-icon.png`: 256 px full bleed, the logo inside the app.
"""

import os

import numpy as np
from PIL import Image, ImageFilter
from scipy import ndimage as ndi

HERE = os.path.dirname(os.path.abspath(__file__))
APP = os.path.dirname(HERE)
SOURCE = os.path.join(APP, "src-tauri/icons-src/source.png")

WHITE = 225  # a pixel at least this light in every channel can be background
DARK = 140  # a pixel at most this light in every channel is outline
GAP = 8  # outlines are thickened by this many pixels while filling, closing gaps up to ~2x this
# The square cut from the 1254 px artwork: the whole head, from just below the top edge, where
# the headband runs off the picture and small pockets of background stay unfilled.
CROP = (85, 45, 1160, 1120)
SKY_TOP_RIGHT = (158, 206, 248)
SKY_BOTTOM_LEFT = (88, 150, 228)


def cut_out(rgb: np.ndarray) -> Image.Image:
    """The drawing with its white background made transparent."""
    white = rgb.min(axis=2) >= WHITE
    dark = rgb.max(axis=2) <= DARK
    # The art runs off the top edge: close that edge too, or the fill enters shapes open there.
    dark[:3, :] = True
    barrier = ndi.binary_dilation(dark, structure=np.ones((3, 3)), iterations=GAP)
    labels, _ = ndi.label(white & ~barrier)
    outside = set(np.unique(np.concatenate([labels[-1], labels[:, 0], labels[:, -1]]))) - {0}
    bg = np.isin(labels, list(outside))
    bg = ndi.binary_dilation(bg, structure=np.ones((3, 3)), iterations=GAP + 2, mask=white)

    alpha = np.where(bg, 0.0, 1.0)
    fg = rgb.astype(np.float32)
    edge = ndi.binary_dilation(bg, iterations=3) & ~bg
    c = fg[edge]
    a = np.clip((255 - c.min(axis=1)) / 255 * 1.4, 0, 1)
    alpha[edge] = a
    fg[edge] = np.clip((c - (1 - a)[:, None] * 255) / np.maximum(a, 1e-3)[:, None], 0, 255)
    return Image.fromarray(np.dstack([fg, alpha * 255]).astype(np.uint8), "RGBA")


def sky(size: int) -> Image.Image:
    y, x = np.mgrid[0:size, 0:size] / (size - 1)
    t = ((1 - x) + y)[..., None] / 2
    g = np.array(SKY_TOP_RIGHT) * (1 - t) + np.array(SKY_BOTTOM_LEFT) * t
    return Image.fromarray(np.dstack([g, np.full((size, size), 255)]).astype(np.uint8), "RGBA")


def squircle(size: int, n: float = 5.0, supersample: int = 4) -> Image.Image:
    """Mask of a superellipse, the continuous-corner shape of app icons."""
    big = size * supersample
    y, x = np.mgrid[0:big, 0:big]
    u = (x + 0.5) / big * 2 - 1
    v = (y + 0.5) / big * 2 - 1
    inside = (np.abs(u) ** n + np.abs(v) ** n) <= 1
    return Image.fromarray((inside * 255).astype(np.uint8)).resize((size, size), Image.LANCZOS)


def full_bleed(art: Image.Image, size: int) -> Image.Image:
    icon = sky(size)
    icon.alpha_composite(art.crop(CROP).resize((size, size), Image.LANCZOS))
    icon.putalpha(squircle(size))
    return icon


def apple_template(art: Image.Image) -> Image.Image:
    body = full_bleed(art, 824)
    canvas = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    shadow = Image.new("RGBA", (1024, 1024), (0, 0, 0, 0))
    shadow.paste((0, 0, 0, 56), (100, 112), squircle(824))
    canvas.alpha_composite(shadow.filter(ImageFilter.GaussianBlur(14)))
    canvas.alpha_composite(body, (100, 100))
    return canvas


def main():
    art = cut_out(np.asarray(Image.open(SOURCE).convert("RGB")))
    full_bleed(art, 1024).save(os.path.join(APP, "src-tauri/icons-src/icon.png"))
    apple_template(art).save(os.path.join(APP, "src-tauri/icons-src/icon-macos.png"))
    full_bleed(art, 256).save(os.path.join(APP, "src/assets/app-icon.png"))
    print("icons-src/icon.png, icons-src/icon-macos.png and src/assets/app-icon.png written")


if __name__ == "__main__":
    main()
