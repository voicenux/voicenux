#!/usr/bin/env python3
"""Downloads the starter sounds from Freesound and makes them ready for the app.

All of them are CC0 (public domain). Each one gets its silence at the start cut, is cut to a
soundboard length with a fade, turns mono at 48 kHz, and is brought to the same loudness:
-16 LUFS, peaks at most -1 dB. The sounds are committed, so this only runs when the list changes.
Needs curl and ffmpeg.
"""

import os
import re
import subprocess
import tempfile

# id | Freesound sound id | preview file | seconds to keep (0: all) | fade out
LIST = """
airhorn|68999|68/68999_533680|3.2|0.8
sad-trombone|175409|175/175409_1326576|0|0
rimshot|132418|132/132418_2412414|0|0
drumroll|424927|424/424927_6842851|7.0|0.8
applause|221567|221/221567_1282865|6.0|1.2
tada|397355|397/397355_4284968|0|0
correct|421002|421/421002_7614679|0|0
wrong|648462|648/648462_11771918|0|0
crickets|146033|146/146033_719177|5.0|1.0
boing|540790|540/540790_11537497|0|0
laser|193427|193/193427_1038806|0|0
explosion|235968|235/235968_4265427|6.0|1.2
coin|402067|402/402067_6142149|0|0
whoosh|60013|60/60013_71257|0|0
"""

TARGET, PEAK = -16.0, -1.0
out = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "data", "sounds")
os.makedirs(out, exist_ok=True)

with tempfile.TemporaryDirectory() as tmp:
    for line in LIST.strip().splitlines():
        sid, _, preview, keep, fade = line.split("|")
        raw = os.path.join(tmp, sid + ".ogg")
        url = f"https://cdn.freesound.org/previews/{preview}-hq.ogg"
        subprocess.run(["curl", "-fsSL", "-o", raw, url], check=True)
        chain = ["silenceremove=start_periods=1:start_threshold=-55dB", "aresample=48000", "pan=mono|c0=0.5*c0+0.5*c1"]
        if float(keep) > 0:
            chain += [f"atrim=0:{keep}", f"afade=t=out:st={float(keep) - float(fade)}:d={fade}"]
        if sid == "crickets":
            chain.append("afade=t=in:d=0.3")
        base = ",".join(chain)
        log = subprocess.run(
            ["ffmpeg", "-v", "info", "-i", raw, "-af", base + ",ebur128=peak=true", "-f", "null", "-"],
            capture_output=True,
            text=True,
        ).stderr
        summary = log[log.rfind("Summary:") :]
        loudness = float(re.search(r"I:\s+(-?[0-9.]+) LUFS", summary)[1])
        peak = float(re.search(r"Peak:\s+(-?[0-9.]+) dBFS", summary)[1])
        gain = min(TARGET - loudness, PEAK - peak)
        dst = os.path.join(out, sid + ".ogg")
        args = ["-af", f"{base},volume={gain:.2f}dB", "-map_metadata", "-1", "-c:a", "libvorbis", "-q:a", "5", dst]
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", raw, *args], check=True)
        print(f"{sid}: {gain:+.1f} dB")
