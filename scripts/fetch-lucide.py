#!/usr/bin/env python3
"""Downloads the Lucide icons (ISC) of the sidebar and the settings.

Every icon in NAMES becomes one line of data/lucide/icons.tsv: its name and its shapes as SVG
path data, the strokes and the few filled dots apart. The app draws them as paths, so they are
sharp at every size and take any color. Everything is committed, so this only runs when NAMES
changes.
"""

import io
import os
import re
import tarfile
import urllib.request
import xml.etree.ElementTree as ET

VERSION = "0.460.0"
PACKAGE = f"https://registry.npmjs.org/lucide-static/-/lucide-static-{VERSION}.tgz"
# The icons the app uses. Find more at https://lucide.dev/icons.
NAMES = """
    bug coffee flask-conical globe headphones keyboard mic mic-vocal plug scissors settings
    sparkles volume-2 zap
""".split()
NUM = r"[-+]?(?:\d+\.?\d*|\.\d+)(?:[eE][-+]?\d+)?"

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
out = os.path.join(root, "data", "lucide")
os.makedirs(out, exist_ok=True)


def num(v):
    s = f"{float(v):.3f}".rstrip("0").rstrip(".")
    return "0" if s == "-0" else s


def shape(el):
    """One SVG element as path data."""
    tag = el.tag.split("}")[-1]
    a = el.attrib
    f = lambda k, d=0.0: float(a.get(k, d))
    if tag == "path":
        # A path starts where its first "m" says. Joined to others, a small "m" would start
        # where the last one ended, so it becomes an "M", and its further points an "l".
        d = a["d"].strip()
        m = re.match(r"m\s*(" + NUM + r")[\s,]*(" + NUM + r")[\s,]*", d)
        if not m:
            return d
        rest = d[m.end():]
        return f"M{m.group(1)} {m.group(2)}" + ("l" if re.match(NUM, rest) else "") + rest
    if tag in ("circle", "ellipse"):
        cx, cy = f("cx"), f("cy")
        rx = f("r") if tag == "circle" else f("rx")
        ry = f("r") if tag == "circle" else f("ry")
        return (f"M{num(cx - rx)} {num(cy)}a{num(rx)} {num(ry)} 0 1 0 {num(2 * rx)} 0"
                f"a{num(rx)} {num(ry)} 0 1 0 {num(-2 * rx)} 0z")
    if tag == "rect":
        x, y, w, h = f("x"), f("y"), f("width"), f("height")
        rx = f("rx", a.get("ry", 0))
        ry = f("ry", a.get("rx", 0))
        rx, ry = min(rx, w / 2), min(ry, h / 2)
        if rx == 0 or ry == 0:
            return f"M{num(x)} {num(y)}h{num(w)}v{num(h)}h{num(-w)}z"
        arc = f"A{num(rx)} {num(ry)} 0 0 1 "
        return (f"M{num(x + rx)} {num(y)}H{num(x + w - rx)}{arc}{num(x + w)} {num(y + ry)}"
                f"V{num(y + h - ry)}{arc}{num(x + w - rx)} {num(y + h)}H{num(x + rx)}"
                f"{arc}{num(x)} {num(y + h - ry)}V{num(y + ry)}{arc}{num(x + rx)} {num(y)}z")
    if tag == "line":
        return f"M{num(f('x1'))} {num(f('y1'))}L{num(f('x2'))} {num(f('y2'))}"
    if tag in ("polyline", "polygon"):
        p = [num(v) for v in re.split(r"[\s,]+", a["points"].strip())]
        pts = [f"{p[i]} {p[i + 1]}" for i in range(0, len(p), 2)]
        return "M" + "L".join(pts) + ("z" if tag == "polygon" else "")
    raise ValueError(f"unknown element {tag}")


with urllib.request.urlopen(PACKAGE, timeout=60) as r:
    tar = tarfile.open(fileobj=io.BytesIO(r.read()))
read = lambda name: tar.extractfile(tar.getmember(name)).read()
with open(os.path.join(out, "LICENSE"), "wb") as f:
    f.write(read("package/LICENSE"))

lines = [f"# name, stroke path, fill path. Lucide {VERSION} (ISC), made by scripts/fetch-lucide.py."]
for icon in sorted(NAMES):
    svg = ET.fromstring(read(f"package/icons/{icon}.svg"))
    strokes, fills = [], []
    for el in svg.iter():
        if el is svg:
            continue
        d = shape(el)
        strokes.append(d)
        if el.attrib.get("fill", "none") != "none":
            fills.append(d)
    lines.append("\t".join([icon, " ".join(strokes), " ".join(fills)]))

with open(os.path.join(out, "icons.tsv"), "w") as f:
    f.write("\n".join(lines) + "\n")
print(len(lines) - 1, "icons")
