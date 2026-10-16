#!/usr/bin/env python3
"""Makes the pictures of the AI voices: Fluent Emoji 3D (MIT, Microsoft) in the colors of the
characters, like a pink star for Patrick or a green old man for Yoda. Only colors change, nothing
is drawn. They go to data/icons and are committed, so this only runs to change them. Needs Pillow
and NumPy.
"""

import io
import os
import urllib.parse
import urllib.request

import numpy as np
from PIL import Image, ImageFilter

FLUENT = "https://raw.githubusercontent.com/microsoft/fluentui-emoji/main/assets/"
root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
out = os.path.join(root, "data", "icons")


def fluent(path):
    with urllib.request.urlopen(FLUENT + urllib.parse.quote(path), timeout=60) as r:
        img = Image.open(io.BytesIO(r.read())).convert("RGBA")
    return np.asarray(img).astype(np.float32) / 255


def to_hsv(rgb):
    mx, mn = rgb.max(-1), rgb.min(-1)
    d = mx - mn
    r, g, b = rgb[..., 0], rgb[..., 1], rgb[..., 2]
    safe = np.where(d > 1e-6, d, 1)
    h = np.where(mx == r, ((g - b) / safe) % 6, np.where(mx == g, (b - r) / safe + 2, (r - g) / safe + 4))
    h = np.where(d > 1e-6, h * 60, 0)
    s = np.where(mx > 1e-6, d / np.maximum(mx, 1e-6), 0)
    return np.stack([h, s, mx], -1)


def to_rgb(hsv):
    h, s, v = hsv[..., 0] % 360 / 60, hsv[..., 1], hsv[..., 2]
    c = v * s
    x = c * (1 - np.abs(h % 2 - 1))
    z = np.zeros_like(h)
    i = np.floor(h).astype(int) % 6
    r = np.choose(i, [c, x, z, z, x, c])
    g = np.choose(i, [x, c, c, x, z, z])
    b = np.choose(i, [z, z, x, c, c, x])
    return np.stack([r, g, b], -1) + (v - c)[..., None]


def soft(mask, radius):
    """Blurs a 0..1 mask, so changes fade out instead of ending in a hard edge."""
    img = Image.fromarray((np.clip(mask, 0, 1) * 255).astype(np.uint8))
    return np.asarray(img.filter(ImageFilter.GaussianBlur(radius))).astype(np.float32) / 255


def ellipse(shape, cx, cy, rx, ry):
    """A mask: 1 inside the ellipse, positions as parts of the picture."""
    h, w = shape[:2]
    y, x = np.mgrid[0:h, 0:w]
    return (((x / w - cx) / rx) ** 2 + ((y / h - cy) / ry) ** 2 <= 1).astype(np.float32)


def hues(a, center, width, min_sat=0.15):
    """A mask of the colors near `center` on the color wheel, soft at its edge."""
    hsv = to_hsv(a[..., :3])
    d = np.abs((hsv[..., 0] - center + 180) % 360 - 180)
    return np.clip((width + 15 - d) / 15, 0, 1) * np.clip((hsv[..., 1] - min_sat) / 0.1, 0, 1)


def grays(a, max_sat=0.12):
    hsv = to_hsv(a[..., :3])
    return np.clip((max_sat - hsv[..., 1]) / 0.05, 0, 1)


def paint(a, mask, hue=None, turn=None, sat=None, sat_mul=1.0, val_mul=1.0):
    """Recolors where `mask` is: to `hue`, or turned by `turn` degrees, with more or less
    saturation and brightness. Shading stays, so it still looks 3D."""
    hsv = to_hsv(a[..., :3])
    new = hsv.copy()
    if hue is not None:
        new[..., 0] = hue
    if turn is not None:
        new[..., 0] = hsv[..., 0] + turn
    new[..., 1] = np.clip((sat if sat is not None else hsv[..., 1]) * sat_mul, 0, 1)
    new[..., 2] = np.clip(hsv[..., 2] * val_mul, 0, 1)
    m = mask[..., None]
    b = a.copy()
    b[..., :3] = a[..., :3] * (1 - m) + to_rgb(new) * m
    return b


def spongebob():
    a = fluent("Sponge/3D/sponge_3d.png")
    return paint(a, hues(a, 25, 40), hue=50, sat_mul=1.05, val_mul=1.05)


def patrick():
    a = fluent("Star/3D/star_3d.png")
    return paint(a, hues(a, 48, 30), turn=-68, sat_mul=0.55)


def squidward():
    a = fluent("Octopus/3D/octopus_3d.png")
    return paint(a, hues(a, 0, 60), turn=168, sat_mul=0.7, val_mul=0.88)


def darth_vader():
    a = fluent("Ninja/Default/3D/ninja_3d_default.png")
    # A black mask instead of the face, and a red blade.
    face = hues(a, 35, 30, min_sat=0.2) * soft(ellipse(a.shape, 0.58, 0.27, 0.26, 0.14), 2)
    a = paint(a, face, sat=0.0, val_mul=0.28)
    blade = grays(a, 0.4) * soft(ellipse(a.shape, 0.16, 0.22, 0.17, 0.24), 2)
    return paint(a, blade, hue=0, sat=0.9, val_mul=2.2)


def yoda():
    a = fluent("Old man/Light/3D/old_man_3d_light.png")
    return paint(a, hues(a, 25, 20, min_sat=0.08), hue=85, sat_mul=1.6, val_mul=0.95)


def optimus_prime():
    a = fluent("Robot/3D/robot_3d.png")
    return paint(a, grays(a, 0.35), hue=356, sat=0.8, val_mul=0.9)


def glados():
    a = fluent("Eye/3D/eye_3d.png")
    iris = soft(hues(a, 25, 30, min_sat=0.25), 1.5)
    return paint(a, iris, hue=42, sat_mul=1.6, val_mul=1.8)


def rick_sanchez():
    a = fluent("Man scientist/Light/3D/man_scientist_3d_light.png")
    hair = grays(a, 0.35) * soft(ellipse(a.shape, 0.5, 0.12, 0.3, 0.16), 2)
    return paint(a, hair, hue=200, sat=0.3, val_mul=3.2)


def homer_simpson():
    a = fluent("Man bald/Default/3D/man_bald_3d_default.png")
    # Stubble around the mouth.
    jaw = hues(a, 42, 20, min_sat=0.3) * soft(ellipse(a.shape, 0.5, 0.84, 0.3, 0.2), 6)
    return paint(a, jaw, hue=35, sat_mul=0.22, val_mul=0.78)


PICTURES = {
    "spongebob": spongebob,
    "patrick": patrick,
    "squidward": squidward,
    "darth-vader": darth_vader,
    "yoda": yoda,
    "optimus-prime": optimus_prime,
    "glados": glados,
    "rick-sanchez": rick_sanchez,
    "homer-simpson": homer_simpson,
}

if __name__ == "__main__":
    import sys

    target = sys.argv[1] if len(sys.argv) > 1 else out
    os.makedirs(target, exist_ok=True)
    for name, make in PICTURES.items():
        a = make()
        Image.fromarray((np.clip(a, 0, 1) * 255).astype(np.uint8)).save(os.path.join(target, f"{name}.png"), optimize=True)
        print(name)
