#!/usr/bin/env python3
"""Downloads all Fluent Emoji 3D pictures (MIT, Microsoft) for the picture picker.

The pictures that voices and sounds use come from fetch-icons.sh in full size. All others go to
data/emoji at 160 px with 256 colors, which looks the same and keeps the app small. data/emoji.tsv
lists every picture in emoji order, with English names from Fluent and German names from Unicode
CLDR, for the search. Everything is committed, so this only runs to update. Needs Pillow.
"""

import concurrent.futures
import io
import json
import os
import re
import urllib.parse
import urllib.request

from PIL import Image

FLUENT = "https://raw.githubusercontent.com/microsoft/fluentui-emoji/main/"
TREE = "https://api.github.com/repos/microsoft/fluentui-emoji/git/trees/main?recursive=1"
CLDR = "https://raw.githubusercontent.com/unicode-org/cldr-json/main/cldr-json/"
CLDR_DE = [
    ("annotations", CLDR + "cldr-annotations-full/annotations/de/annotations.json"),
    ("annotationsDerived", CLDR + "cldr-annotations-derived-full/annotationsDerived/de/annotations.json"),
]
ORDER = "https://unicode.org/Public/emoji/latest/emoji-test.txt"
SIZE = 160

root = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..")
out = os.path.join(root, "data", "emoji")
os.makedirs(out, exist_ok=True)


def get(url):
    with urllib.request.urlopen(url, timeout=60) as r:
        return r.read()


def fluent(path):
    return get(FLUENT + urllib.parse.quote(path))


# The pictures from fetch-icons.sh keep their names: voices and sounds use them.
picked = {}
with open(os.path.join(root, "scripts", "fetch-icons.sh")) as f:
    for line in f.read().split("<<'LIST'")[1].split("LIST")[0].strip().splitlines():
        pid, path = line.split("|")
        picked[path.split("/")[0]] = pid

pngs = {}
for e in json.loads(get(TREE))["tree"]:
    m = re.match(r"assets/([^/]+)/(Default/)?3D/[^/]+\.png$", e["path"])
    if m:
        pngs[m.group(1)] = e["path"].split("/", 1)[1]

with concurrent.futures.ThreadPoolExecutor(16) as pool:
    metas = dict(zip(pngs, pool.map(lambda d: json.loads(fluent(f"assets/{d}/metadata.json")), pngs)))

german = {}
for key, url in CLDR_DE:
    for glyph, a in json.loads(get(url))[key]["annotations"].items():
        german[glyph.replace("\ufe0f", "")] = a


def slug(name):
    return re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")


def key(codes):
    return " ".join(c for c in codes.lower().split() if c != "fe0f")


# The order of emoji keyboards: smileys first, flags last.
place = {}
for line in get(ORDER).decode().splitlines():
    if ";" in line and not line.startswith("#"):
        place.setdefault(key(line.split(";")[0]), len(place))


def order(d):
    return place.get(key(metas[d]["unicode"]), len(place))


ids = set(picked.values())
folders = sorted(pngs, key=order)
for d in folders:
    if d in picked:
        continue
    i, n = slug(d), 2
    while i in ids:
        i, n = f"{slug(d)}-{n}", n + 1
    ids.add(i)
    picked[d] = i


def shrink(d):
    path = os.path.join(out, picked[d] + ".png")
    if os.path.exists(path):
        return
    im = Image.open(io.BytesIO(fluent("assets/" + pngs[d]))).convert("RGBA")
    im = im.resize((SIZE, SIZE), Image.LANCZOS)
    im.quantize(256, method=Image.Quantize.LIBIMAGEQUANT, dither=Image.Dither.FLOYDSTEINBERG).save(path, optimize=True)


icons = {f[:-4] for f in os.listdir(os.path.join(root, "data", "icons"))}
with concurrent.futures.ThreadPoolExecutor(16) as pool:
    list(pool.map(shrink, [d for d in folders if picked[d] not in icons]))

with open(os.path.join(root, "data", "emoji.tsv"), "w") as f:
    f.write("# id, English name, German name, search words. Made by scripts/fetch-emoji.py.\n")
    for d in folders:
        m = metas[d]
        de = german.get(m["glyph"].replace("\ufe0f", ""), {})
        en_name = m["cldr"][:1].upper() + m["cldr"][1:]
        de_name = (de.get("tts") or [en_name])[0]
        words = [picked[d].replace("-", " "), m["cldr"], m["tts"], *m["keywords"], de_name, *de.get("default", [])]
        seen = []
        for w in " ".join(words).lower().split():
            if w not in seen:
                seen.append(w)
        f.write(f"{picked[d]}\t{en_name}\t{de_name}\t{' '.join(seen)}\n")
print(len(folders), "pictures")
