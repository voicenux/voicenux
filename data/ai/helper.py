"""The AI helper of Voicenux: turns a live voice into a character with an RVC model.

Voicenux installs it with its own Python (see src/ai/install.rs) and runs it next to the engine.
They talk over stdin and stdout in frames: one byte for the kind, four bytes of length (little
endian), then the payload. Log lines go to stderr.

From Voicenux:
  L  load a model: JSON {"model": path of its .pth file, "rate": sample rate of the engine}
  A  audio: f32 pitch shift in semitones, then one block of mono f32 samples
To Voicenux:
  R  ready: JSON {"device": "cuda", "block": seconds per block}
  M  model loaded: JSON {"model": path, "rate": sample rate of the model}
  A  audio: mono f32 samples, about one block
  E  error: text

`python helper.py prepare --models DIR` only downloads the models every voice needs.

It works like the real-time mode of RVC: each block joins a window with a second of the voice
before it, the whole window goes through HuBERT and the pitch finder, and the model renders only
the newest part. The parts are joined where their waves line up best, with a short crossfade.
"""

import argparse
import json
import os
import struct
import sys

import numpy as np

HUBERT_RATE = 16000
# 100 frames a second, like RVC.
HOP = 160
# Below this the input is silence: the model would make breathing noise of it.
SILENCE = 10 ** (-50 / 20)
F0_MIN, F0_MAX = 50.0, 1100.0
MEL_MIN = 1127 * np.log(1 + F0_MIN / 700)
MEL_MAX = 1127 * np.log(1 + F0_MAX / 700)
RMVPE_URL = "https://huggingface.co/r3gm/sonitranslate_voice_models/resolve/main/rmvpe.pt"

# Seconds per block, of voice before it, of crossfade, and of room to find the best join.
TIMING = {
    "cuda": dict(block=0.25, extra=1.0, fade=0.05, search=0.012),
    "cpu": dict(block=0.5, extra=0.6, fade=0.05, search=0.012),
}


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
        kind = "cuda" if self.device.startswith("cuda") else "cpu"
        t = TIMING[kind]
        self.kind = kind
        self.block_s, self.fade_s, self.search_s = t["block"], t["fade"], t["search"]
        window = t["extra"] + t["fade"] + t["search"] + t["block"]
        self.window16 = int(np.ceil(window * HUBERT_RATE / HOP)) * HOP
        self.net = None

    def load(self, path, rate):
        from infer_rvc_python.main import load_trained_model
        import soxr

        _, self.sr, self.net, _, cpt, self.version = load_trained_model(path, self.config)
        self.pitched = cpt.get("f0", 1) == 1
        self.down = soxr.ResampleStream(rate, HUBERT_RATE, 1, dtype="float32")
        self.up = soxr.ResampleStream(self.sr, rate, 1, dtype="float32")
        self.buf = np.zeros(self.window16, dtype=np.float32)
        self.fade = int(self.fade_s * self.sr)
        self.search = int(self.search_s * self.sr)
        self.block = int(round(self.block_s * self.sr))
        ramp = np.sin(0.5 * np.pi * np.linspace(0.0, 1.0, self.fade, dtype=np.float32)) ** 2
        self.fade_in, self.fade_out = ramp, 1.0 - ramp
        self.prev = None
        # 10 ms frames the model renders each time: the block, the crossfade, the search, one more.
        self.frames = int(np.ceil((self.block_s + self.fade_s + self.search_s) * 100)) + 1
        return self.sr

    def convert(self, x, key):
        x16 = self.down.resample_chunk(x)
        if len(x16):
            self.buf = np.concatenate([self.buf[len(x16):], x16.astype(np.float32)])
        if np.sqrt(np.mean(x * x)) < SILENCE:
            audio = np.zeros(self.frames * (self.sr // 100), dtype=np.float32)
        else:
            audio = self.render(key)
        out = self.join(audio)
        return self.up.resample_chunk(out)

    def render(self, key):
        torch = self.torch
        with torch.no_grad():
            src = torch.from_numpy(self.buf).to(self.device)
            src = src.half() if self.half else src.float()
            hidden = self.hubert.extract_features(src.view(1, -1), output_layer=9 if self.version == "v1" else 12)[0]
            feats = self.hubert.final_proj(hidden) if self.version == "v1" else hidden
            feats = torch.nn.functional.interpolate(feats.permute(0, 2, 1), scale_factor=2).permute(0, 2, 1)
            p_len = min(len(self.buf) // HOP, feats.shape[1])
            feats = feats[:, :p_len]
            sid = torch.tensor([0], device=self.device).long()
            rate = (self.frames + 0.5) / p_len
            length = torch.tensor([p_len], device=self.device).long()
            if self.pitched:
                f0 = self.rmvpe.infer_from_audio(self.buf, thred=0.03)
                f0 = np.pad(f0, (0, max(0, p_len - len(f0))))[:p_len] * 2 ** (key / 12)
                mel = 1127 * np.log(1 + f0 / 700)
                mel[mel > 0] = (mel[mel > 0] - MEL_MIN) * 254 / (MEL_MAX - MEL_MIN) + 1
                coarse = np.rint(np.clip(mel, 1, 255)).astype(np.int64)
                pitch = torch.tensor(coarse, device=self.device).unsqueeze(0)
                pitchf = torch.tensor(f0.astype(np.float32), device=self.device).unsqueeze(0)
                audio = self.net.infer(feats, length, pitch, pitchf, sid, rate=rate)[0][0, 0]
            else:
                audio = self.net.infer(feats, length, sid, rate=rate)[0][0, 0]
            return audio.float().cpu().numpy()

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
    send(b"R", json.dumps({"device": conv.kind, "block": conv.block_s}).encode())
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
                rate = conv.load(order["model"], order["rate"])
                send(b"M", json.dumps({"model": order["model"], "rate": rate}).encode())
                log("loaded", order["model"], rate)
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
