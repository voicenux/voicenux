"""The AI helper of Voicenux: turns a live voice into a character with an RVC model.

Voicenux installs it with its own Python (see src/ai/install.rs) and runs it next to the engine.
They talk over stdin and stdout in frames: one byte for the kind, four bytes of length (little
endian), then the payload. Log lines go to stderr.

From Voicenux:
  L  load a model: JSON {"model": path of its .pth file, "index": path of its .index file or
     null, "pitch": typical pitch of the character in Hz, "rate": sample rate of the engine}
  A  audio: f32 pitch shift in semitones on top of the character's pitch, then one block of
     mono f32 samples
To Voicenux:
  R  ready: JSON {"device": "cuda"}
  M  model loaded: JSON {"model": path, "rate": sample rate of the model, "block": seconds per
     block, "cushion": seconds Voicenux keeps in store against late blocks, "delay": about how
     many seconds the voice comes later}
  A  audio: mono f32 samples, one block
  E  error: text

`python helper.py prepare --models DIR` only downloads the models every voice needs.

It works like the real-time mode of RVC: each block joins a window with the voice before it, the
whole window goes through HuBERT and the pitch finder, and the model renders only the newest
part. The parts are joined where their waves line up best, with a short crossfade. The index of
the model swaps the speech features for the nearest ones from the character's own recordings,
which makes it sound more like the character. Your typical pitch, learned while you talk, moves
to the character's, so the voice fits deep and high voices alike.
"""

import argparse
import json
import os
import struct
import sys
import time

import numpy as np

HUBERT_RATE = 16000
# 100 frames a second, like RVC.
HOP = 160
# Below this the input is silence: the model would make breathing noise of it. Only after this
# many seconds of silence, so quiet bits of speech go through.
SILENCE = 10 ** (-60 / 20)
HOLD = 0.4
F0_MIN, F0_MAX = 50.0, 1100.0
MEL_MIN = 1127 * np.log(1 + F0_MIN / 700)
MEL_MAX = 1127 * np.log(1 + F0_MAX / 700)
RMVPE_URL = "https://huggingface.co/r3gm/sonitranslate_voice_models/resolve/main/rmvpe.pt"

# Seconds of voice before each block, of crossfade, and of room to find the best join.
TIMING = {
    "cuda": dict(extra=1.0, fade=0.03, search=0.01),
    "cpu": dict(extra=0.6, fade=0.04, search=0.01),
}
# Seconds of the window HuBERT also gets mirrored at both ends (see `render`).
MIRROR = 0.6
# Seconds per block: the smallest the device renders in 40 % of its time. Smaller blocks mean
# less delay, but below 120 ms a held "aaa" wobbles between vowels.
BLOCKS = (0.12, 0.16, 0.2, 0.25, 0.32, 0.4, 0.5)
# How much of the speech features come from the index.
INDEX_RATE = 0.75
# Your typical pitch in Hz until Voicenux has heard you, and how many 10 ms frames of your
# voice it keeps to learn it: 30 s.
FIRST_GUESS = 120.0
HEARD = 3000


def log(*args):
    print("ai:", *args, file=sys.stderr, flush=True)


def rmvpe_path(models):
    path = os.path.join(models, "rmvpe.pt")
    if not os.path.exists(path):
        from torch.hub import download_url_to_file

        os.makedirs(models, exist_ok=True)
        log("downloading", RMVPE_URL)
        download_url_to_file(RMVPE_URL, path + ".part", progress=False)
        os.replace(path + ".part", path)
    return path


class Converter:
    def __init__(self, device, models):
        import torch
        from infer_rvc_python.lib.rmvpe import RMVPE
        from infer_rvc_python.main import Config, load_hu_bert

        self.torch = torch
        self.config = Config(only_cpu=device == "cpu")
        self.device = self.config.device
        self.half = self.config.is_half
        self.hubert = load_hu_bert(self.config)
        self.rmvpe = RMVPE(rmvpe_path(models), is_half=self.half, device=self.device)
        self.kind = "cuda" if self.device.startswith("cuda") else "cpu"
        self.net = None
        self.index = None
        self.heard = np.zeros(0, dtype=np.float32)
        self.quiet = HOLD

    def load(self, path, index, pitch, rate):
        """Loads a model, warms it up and picks the block size. Returns the block and cushion."""
        from infer_rvc_python.main import load_trained_model

        _, self.sr, self.net, _, cpt, self.version = load_trained_model(path, self.config)
        self.pitched = cpt.get("f0", 1) == 1
        self.pitch = pitch
        self.load_index(index)
        # The first runs are slow while the device gets ready: never let a voice wait for them.
        self.setup(0.1, rate)
        for _ in range(5):
            self.render(0.0, learn=False)
        times = []
        for _ in range(8):
            t0 = time.perf_counter()
            self.render(0.0, learn=False)
            times.append(time.perf_counter() - t0)
        took, worst = float(np.median(times)), max(times)
        block = next((b for b in BLOCKS if b >= 2.5 * took), BLOCKS[-1])
        self.setup(block, rate)
        self.render(0.0, learn=False)
        cushion = max(0.04, 0.3 * block, 2 * (worst - took))
        log(f"renders in {took * 1000:.0f} ms, block {block * 1000:.0f} ms, cushion {cushion * 1000:.0f} ms")
        # The block fills, renders and waits in the cushion. The join holds back the crossfade
        # and the search, the resamplers and PipeWire add a little.
        delay = block + took + cushion + self.fade_s + self.search_s + 0.05
        return block, cushion, delay

    def load_index(self, path):
        self.index = self.big = None
        if not path or not os.path.exists(path):
            return
        import faiss

        index = faiss.read_index(path)
        if index.ntotal == 0:
            return
        try:
            big = index.reconstruct_n(0, index.ntotal)
        except RuntimeError:
            faiss.extract_index_ivf(index).make_direct_map()
            big = index.reconstruct_n(0, index.ntotal)
        self.index, self.big = index, big

    def setup(self, block, rate):
        import soxr

        t = TIMING[self.kind]
        self.block_s, self.fade_s, self.search_s = block, t["fade"], t["search"]
        window = t["extra"] + t["fade"] + t["search"] + block
        # Whole HuBERT frames of 20 ms.
        self.window16 = int(np.ceil(window * HUBERT_RATE / (2 * HOP))) * 2 * HOP
        self.down = soxr.ResampleStream(rate, HUBERT_RATE, 1, dtype="float32")
        self.up = soxr.ResampleStream(self.sr, rate, 1, dtype="float32")
        # The resampler gives a little more or less each time: send exactly a block anyway.
        self.out_block = int(round(block * rate))
        self.pending = np.zeros(0, dtype=np.float32)
        # A buzz like a voice, so the warm-up takes as long as real speech.
        n = np.arange(self.window16)
        self.buf = (0.1 * ((n * 120 / HUBERT_RATE) % 1.0 - 0.5)).astype(np.float32)
        self.fade = int(self.fade_s * self.sr)
        self.search = int(self.search_s * self.sr)
        self.block = int(round(self.block_s * self.sr))
        ramp = np.sin(0.5 * np.pi * np.linspace(0.0, 1.0, self.fade, dtype=np.float32)) ** 2
        self.fade_in, self.fade_out = ramp, 1.0 - ramp
        self.prev = None
        # 10 ms frames the model renders each time: the block, the crossfade, the search, one more.
        self.frames = int(np.ceil((self.block_s + self.fade_s + self.search_s) * 100)) + 1

    def start(self):
        """Forgets the warm-up: the voice starts from silence."""
        self.buf[:] = 0.0
        self.prev = None
        self.quiet = HOLD
        self.pending = np.zeros(0, dtype=np.float32)

    def convert(self, x, key):
        x16 = self.down.resample_chunk(x)
        if len(x16):
            self.buf = np.concatenate([self.buf[len(x16):], x16.astype(np.float32)])
        self.quiet = self.quiet + self.block_s if np.sqrt(np.mean(x * x)) < SILENCE else 0.0
        if self.quiet >= HOLD:
            audio = np.zeros(self.frames * (self.sr // 100), dtype=np.float32)
        else:
            audio = self.render(key)
        out = self.join(audio)
        self.pending = np.concatenate([self.pending, self.up.resample_chunk(out).astype(np.float32)])
        n = self.out_block
        if len(self.pending) < n:
            # Only at the start, while the resampler holds a little back.
            self.pending = np.concatenate([np.zeros(n - len(self.pending), dtype=np.float32), self.pending])
        out, self.pending = self.pending[:n], self.pending[n:]
        return out

    def shift(self, key):
        """Semitones from your typical pitch to the character's, plus `key`."""
        if not self.pitch:
            return key
        mine = float(np.median(self.heard)) if len(self.heard) >= 200 else FIRST_GUESS
        return 12 * np.log2(self.pitch / mine) + key

    def render(self, key, learn=True):
        torch = self.torch
        with torch.no_grad():
            # HuBERT looks more than a second to both sides. Past the ends of the window it would
            # see silence, and the newest frames would change from block to block: a held "aaa"
            # jumps between vowels. Mirrored voice there keeps them steady.
            pad = int(MIRROR * HUBERT_RATE) // (2 * HOP) * 2 * HOP
            src = np.concatenate([self.buf[:pad][::-1], self.buf, self.buf[::-1][:pad]])
            src = torch.from_numpy(src).to(self.device)
            src = src.half() if self.half else src.float()
            hidden = self.hubert.extract_features(src.view(1, -1), output_layer=9 if self.version == "v1" else 12)[0]
            hidden = hidden[:, pad // (2 * HOP) : (pad + len(self.buf)) // (2 * HOP)]
            feats = self.hubert.final_proj(hidden) if self.version == "v1" else hidden
            if self.index is not None:
                feats = self.retrieve(feats)
            feats = torch.nn.functional.interpolate(feats.permute(0, 2, 1), scale_factor=2).permute(0, 2, 1)
            p_len = min(len(self.buf) // HOP, feats.shape[1])
            feats = feats[:, :p_len]
            sid = torch.tensor([0], device=self.device).long()
            rate = (self.frames + 0.5) / p_len
            length = torch.tensor([p_len], device=self.device).long()
            if self.pitched:
                f0 = self.rmvpe.infer_from_audio(self.buf, thred=0.03)
                f0 = np.pad(f0, (0, max(0, p_len - len(f0))))[:p_len]
                if learn:
                    new = f0[-int(round(self.block_s * 100)) :]
                    self.heard = np.concatenate([self.heard, new[new > 0]])[-HEARD:]
                f0 = f0 * 2 ** (self.shift(key) / 12)
                mel = 1127 * np.log(1 + f0 / 700)
                mel[mel > 0] = (mel[mel > 0] - MEL_MIN) * 254 / (MEL_MAX - MEL_MIN) + 1
                coarse = np.rint(np.clip(mel, 1, 255)).astype(np.int64)
                pitch = torch.tensor(coarse, device=self.device).unsqueeze(0)
                pitchf = torch.tensor(f0.astype(np.float32), device=self.device).unsqueeze(0)
                audio = self.net.infer(feats, length, pitch, pitchf, sid, rate=rate)[0][0, 0]
            else:
                audio = self.net.infer(feats, length, sid, rate=rate)[0][0, 0]
            return audio.float().cpu().numpy()

    def retrieve(self, feats):
        """Mixes the newest features with their nearest ones in the index, like RVC does."""
        # HuBERT runs at 50 frames a second, the model renders at 100.
        n = min(feats.shape[1], self.frames // 2 + 2)
        mine = feats[0, -n:].float().cpu().numpy()
        score, ix = self.index.search(mine, 8)
        if (ix < 0).any():
            return feats
        weight = np.square(1 / np.maximum(score, 1e-9))
        weight /= weight.sum(axis=1, keepdims=True)
        found = np.sum(self.big[ix] * weight[..., None], axis=1)
        found = self.torch.from_numpy(found).to(self.device).to(feats.dtype)
        feats = feats.clone()
        feats[0, -n:] = found * INDEX_RATE + feats[0, -n:] * (1 - INDEX_RATE)
        return feats

    def join(self, audio):
        """Joins the new part to the last one where their waves line up, and keeps its end."""
        fade, search, block = self.fade, self.search, self.block
        off = 0
        if self.prev is not None and len(audio) >= search + fade + block:
            head = audio[: search + fade]
            num = np.correlate(head, self.prev, "valid")
            den = np.sqrt(np.convolve(head * head, np.ones(fade, dtype=np.float32), "valid")) + 1e-8
            off = int(np.argmax(num / den))
        audio = audio[off:]
        if len(audio) < block + fade:
            audio = np.pad(audio, (0, block + fade - len(audio)))
        out = audio[:block].copy()
        if self.prev is not None:
            out[:fade] = out[:fade] * self.fade_in + self.prev * self.fade_out
        self.prev = audio[block : block + fade].copy()
        return out


def read_exact(stream, n):
    data = b""
    while len(data) < n:
        part = stream.read(n - len(data))
        if not part:
            return None
        data += part
    return data


def send(kind, payload):
    out = sys.stdout.buffer
    out.write(kind + struct.pack("<I", len(payload)) + payload)
    out.flush()


def serve(device, models):
    try:
        conv = Converter(device, models)
    except Exception as e:  # noqa: BLE001 - the reason goes to Voicenux
        send(b"E", f"could not start: {e}".encode())
        raise
    send(b"R", json.dumps({"device": conv.kind}).encode())
    log("ready on", conv.device)
    stdin = sys.stdin.buffer
    while True:
        head = read_exact(stdin, 5)
        if head is None:
            return
        kind, n = head[:1], struct.unpack("<I", head[1:])[0]
        payload = read_exact(stdin, n)
        if payload is None:
            return
        try:
            if kind == b"L":
                order = json.loads(payload)
                block, cushion, delay = conv.load(order["model"], order.get("index"), order.get("pitch"), order["rate"])
                conv.start()
                info = {"model": order["model"], "rate": conv.sr, "block": block, "cushion": cushion, "delay": delay}
                send(b"M", json.dumps(info).encode())
                log("loaded", order["model"], "with index" if conv.index is not None else "without index")
            elif kind == b"A" and conv.net is not None:
                key = struct.unpack("<f", payload[:4])[0]
                x = np.frombuffer(payload[4:], dtype="<f4")
                send(b"A", conv.convert(x, key).astype("<f4").tobytes())
        except Exception as e:  # noqa: BLE001 - keep serving, Voicenux shows the error
            log("error:", repr(e))
            send(b"E", str(e).encode())


def prepare(models):
    """Downloads HuBERT and RMVPE once, while Voicenux shows its install progress."""
    from infer_rvc_python.main import Config, load_hu_bert

    load_hu_bert(Config(only_cpu=True))
    rmvpe_path(models)
    log("prepared")


def main():
    p = argparse.ArgumentParser()
    p.add_argument("command", choices=["serve", "prepare"])
    p.add_argument("--device", default="cuda", choices=["cuda", "cpu"])
    p.add_argument("--models", required=True)
    a = p.parse_args()
    if a.command == "prepare":
        prepare(a.models)
    else:
        serve(a.device, a.models)


if __name__ == "__main__":
    main()
