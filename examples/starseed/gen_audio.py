"""Starseed's music, made here (no samples): a slow pad and a plucked
arpeggio per stage, 32 s loops, 22.05 kHz mono 16-bit WAV."""
import math, os, struct, sys, wave

RATE = 22050
SECONDS = 32
STAGES = {
    1: dict(root=220.0, chord=[0, 4, 7, 11], bpm=84, bright=0.6),
    2: dict(root=196.0, chord=[0, 3, 7, 10], bpm=96, bright=0.8),
    3: dict(root=174.6, chord=[0, 3, 6, 10], bpm=112, bright=1.0),
}
PROGRESSION = [0, 5, -3, 2]  # semitones, one chord per 8 s


def note(root, semis):
    return root * 2 ** (semis / 12)


def render(stage, cfg):
    n = RATE * SECONDS
    out = [0.0] * n
    bar = n // len(PROGRESSION)
    for b, shift in enumerate(PROGRESSION):
        freqs = [note(cfg["root"], s + shift) for s in cfg["chord"]]
        # Pad: detuned sines, slow swell.
        for i in range(bar):
            t = i / RATE
            env = min(1.0, t / 1.5) * min(1.0, (bar - i) / RATE / 1.5)
            v = 0.0
            for f in freqs:
                v += math.sin(2 * math.pi * f * 0.5 * t) + 0.6 * math.sin(2 * math.pi * f * 0.5 * 1.004 * t)
            out[b * bar + i] += 0.07 * env * v
        # Arpeggio: plucks up and down the chord.
        step = int(RATE * 60 / cfg["bpm"] / 2)
        k = 0
        for start in range(0, bar, step):
            f = freqs[[0, 1, 2, 3, 2, 1][k % 6]] * 2
            k += 1
            for i in range(min(step * 3, bar - start)):
                t = i / RATE
                env = math.exp(-t * 5.0)
                v = math.sin(2 * math.pi * f * t) + cfg["bright"] * 0.3 * math.sin(4 * math.pi * f * t)
                out[b * bar + start + i] += 0.12 * env * v
    peak = max(abs(x) for x in out) or 1
    return [x / peak * 0.8 for x in out]


def main(dir):
    os.makedirs(dir, exist_ok=True)
    for stage, cfg in STAGES.items():
        path = os.path.join(dir, f"stage{stage}.wav")
        if os.path.exists(path):
            continue
        with wave.open(path, "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(RATE)
            w.writeframes(b"".join(struct.pack("<h", int(x * 32767)) for x in render(stage, cfg)))
        print(path, os.path.getsize(path))


main(sys.argv[1] if len(sys.argv) > 1 else "site/audio")
