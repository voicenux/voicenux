#!/usr/bin/env python3
"""End-to-end test against the real PipeWire graph.

Makes a fake microphone, starts Voicenux in the background with its own settings
folder, plays a test tone into the fake mic and checks what comes out of the
"Voicenux Microphone". Needs pw-loopback, pw-play, pw-record, pactl and numpy.

    python3 tests/e2e/run.py [path/to/voicenux]
"""

import os
import subprocess
import sys
import tempfile
import time
import wave

import numpy as np

RATE = 48000
BIN = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else "target/release/voicenux")
TMP = tempfile.mkdtemp(prefix="voicenux-e2e-")
ENV = dict(os.environ, XDG_CONFIG_HOME=os.path.join(TMP, "config"), LANGUAGE="en")
procs = []
results = []


def sh(*args, **kw):
    return subprocess.run(args, capture_output=True, text=True, env=ENV, **kw)


def write_wav(path, x):
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        w.writeframes((np.clip(x, -1, 1) * 32767).astype("<i2").tobytes())


def read_wav(path):
    with wave.open(path) as w:
        n, ch, sw = w.getnframes(), w.getnchannels(), w.getsampwidth()
        raw = w.readframes(n)
    x = np.frombuffer(raw, dtype={2: "<i2", 4: "<i4"}[sw]).astype(float).reshape(-1, ch)[:, 0]
    return x / 2 ** (8 * sw - 1)


def voice_signal(seconds=3.0, f0=150.0):
    t = np.arange(int(RATE * seconds)) / RATE
    saw = 2 * ((f0 * t) % 1.0) - 1
    syllables = (0.5 + 0.5 * np.sin(2 * np.pi * 2.5 * t)) ** 2
    return saw * syllables * 0.3


def dominant(x, lo=50, hi=3000):
    seg = x[len(x) // 4: len(x) * 3 // 4]
    sp = np.abs(np.fft.rfft(seg * np.hanning(len(seg))))
    f = np.fft.rfftfreq(len(seg), 1 / RATE)
    m = (f > lo) & (f < hi)
    return f[m][np.argmax(sp[m])]


def rms(x):
    seg = x[len(x) // 4: len(x) * 3 // 4]
    return float(np.sqrt(np.mean(seg ** 2)))


def capture(seconds, play=None, during=None):
    """Records the Voicenux Microphone while playing `play` into the fake mic."""
    out = os.path.join(TMP, f"rec-{time.time_ns()}.wav")
    rec = subprocess.Popen(["pw-record", "--target", "voicenux_mic", "--rate", str(RATE), "--channels", "2", out], env=ENV)
    time.sleep(0.4)
    if during:
        during()
    if play is not None:
        sh("pw-play", "--target", "vn_e2e_sink", play)
    else:
        time.sleep(seconds)
    time.sleep(0.3)
    rec.terminate()
    rec.wait()
    return read_wav(out)


def check(name, ok, detail):
    results.append(ok)
    print(f"{'ok  ' if ok else 'FAIL'} {name}: {detail}")


def main():
    if sh("pgrep", "-x", "voicenux").returncode == 0:
        sys.exit("Voicenux is running already. Quit it first, the test needs its own.")
    module = sh("pactl", "load-module", "module-null-sink", "sink_name=vn_e2e_sink",
                "sink_properties=device.description=E2E-Sink").stdout.strip()
    try:
        procs.append(subprocess.Popen([
            "pw-loopback",
            "--capture-props=target.object=vn_e2e_sink stream.capture.sink=true node.name=vn_e2e_cap",
            "--playback-props=media.class=Audio/Source node.name=vn_e2e_mic node.description=E2E-Mic audio.position=[MONO]",
        ], env=ENV))
        time.sleep(1)
        os.makedirs(ENV["XDG_CONFIG_HOME"] + "/voicenux", exist_ok=True)
        with open(ENV["XDG_CONFIG_HOME"] + "/voicenux/config.toml", "w") as f:
            f.write('onboarded = true\ninput = "vn_e2e_mic"\nvoice = "chipmunk"\nbackground = false\n')
        procs.append(subprocess.Popen([BIN, "--background"], env=ENV, stderr=subprocess.DEVNULL))
        time.sleep(3)

        tone = os.path.join(TMP, "voice.wav")
        x = voice_signal()
        write_wav(tone, x)
        in_rms = rms(x)

        links = sh("pw-link", "-l").stdout
        check("mic is linked", "vn_e2e_mic:capture_MONO" in links and "voicenux_engine:input_MONO" in links, "fake mic feeds the engine")

        y = capture(3, tone)
        f = dominant(y)
        check("chipmunk doubles the pitch", abs(f - 300) < 6, f"{f:.1f} Hz")
        check("level stays close", 0.5 < rms(y) / in_rms < 2.0, f"{20 * np.log10(rms(y) / in_rms):+.1f} dB")

        sh(BIN, "off")
        time.sleep(0.3)
        y = capture(3, tone)
        f = dominant(y)
        check("voice changer off passes the real voice", abs(f - 150) < 4, f"{f:.1f} Hz")
        sh(BIN, "on")

        sh(BIN, "voice", "deep")
        time.sleep(0.5)
        y = capture(3, tone)
        f = dominant(y, 50, 140)
        check("deep voice goes down", abs(f - 150 * 2 ** (-5 / 12)) < 5, f"{f:.1f} Hz")

        sh(BIN, "mute")
        time.sleep(0.3)
        y = capture(3, tone)
        check("mute silences the voice", rms(y) < in_rms * 0.01, f"rms {rms(y):.5f}")
        sh(BIN, "mute")

        def coin():
            sh(BIN, "sound", "coin")

        # The starter coin is a recording, so it is checked by its level, against silence.
        quiet = rms(capture(1.2))
        y = capture(1.2, None, coin)
        check("soundboard plays into the mic", rms(y) > 0.01 and rms(y) > quiet * 10, f"rms {rms(y):.4f}, silence {quiet:.5f}")

        sh(BIN, "voice", "robot")
        time.sleep(0.5)
        y = capture(3, tone)
        check("robot keeps a voice going", rms(y) > in_rms * 0.2, f"rms ratio {rms(y) / in_rms:.2f}")
    finally:
        for p in procs:
            p.terminate()
        for p in procs:
            try:
                p.wait(timeout=5)
            except subprocess.TimeoutExpired:
                p.kill()
        if module:
            sh("pactl", "unload-module", module)
    print(f"\n{sum(results)}/{len(results)} passed")
    sys.exit(0 if all(results) else 1)


if __name__ == "__main__":
    main()
