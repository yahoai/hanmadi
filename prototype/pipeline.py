"""Stage-1 check: sherpa VAD -> per-segment whisper.cpp (lang auto) -> sherpa diarization -> name by language hints.
usage: python3 pipeline.py meeting.wav diar.txt outdir [--cpu] [--limit SEC]
Glue only; the app will redo this in Rust with the same engines.
"""
import itertools, json, re, subprocess, sys, time, wave
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VAD = [str(ROOT / "tools/sherpa/bin/sherpa-onnx-vad"), f"--silero-vad-model={ROOT}/models/silero_vad.onnx"]
WHISPER = [str(ROOT / "tools/whisper.cpp/build/bin/whisper-cli"), "-m", str(ROOT / "models/ggml-large-v3-turbo-q5_0.bin"),
           "-l", "auto", "-oj", "-np", "-t", "8", "-mc", "0", "-sns"]
# who speaks what (user-supplied hint)
HINTS = {"Wei": {"zh"}, "Mina": {"zh", "ko"}, "김민수": {"ko"}}
MIN_SEG = 0.6  # ponytail: drop sub-0.6s blips (whisper hallucinates on them); merge into neighbors if it loses "네" turns


def vad_segments(wav, tmp):
    out = subprocess.run(VAD + [wav, str(tmp / "vad_out.wav")], capture_output=True, text=True)
    segs = [tuple(map(float, m)) for m in re.findall(r"^([\d.]+) -- ([\d.]+)$", out.stdout + out.stderr, re.M)]
    return [s for s in segs if s[1] - s[0] >= MIN_SEG]


def cut(wav, segs, tmp):
    paths = []
    with wave.open(wav) as w:
        sr, data = w.getframerate(), w.readframes(w.getnframes())
    for i, (a, b) in enumerate(segs):
        p = tmp / f"seg{i:04d}.wav"
        with wave.open(str(p), "wb") as o:
            o.setnchannels(1); o.setsampwidth(2); o.setframerate(sr)
            o.writeframes(data[int(a * sr) * 2:int(b * sr) * 2])
        paths.append(p)
    return paths


def diarization(path):
    return [(float(a), float(b), s) for a, b, s in re.findall(r"([\d.]+) -- ([\d.]+) (speaker_\d+)", Path(path).read_text())]


def split_at_turns(segs, diar):
    """VAD keeps quick turn-taking in one segment; cut again where diarization says the speaker changes."""
    out = []
    for a, b in segs:
        cuts, prev = [], None
        for x, y, s in sorted(d for d in diar if d[1] > a and d[0] < b):
            if prev and s != prev and a + MIN_SEG < x < b - MIN_SEG:
                cuts.append(x)
            prev = s
        edges = [a] + cuts + [b]
        out += [(p, q) for p, q in zip(edges, edges[1:]) if q - p >= MIN_SEG]
    return out


def dedupe_loops(text):
    """whisper decoding loops ('对,对,对,...'): keep at most 3 repeats."""
    return re.sub(r"((.{1,12}?)[,，、。 ]*)\1{3,}", r"\1\1\1", text)


def speaker_of(a, b, diar):
    ov = defaultdict(float)
    for x, y, s in diar:
        ov[s] += max(0.0, min(b, y) - max(a, x))
    return max(ov, key=ov.get) if ov and max(ov.values()) > 0 else "?"


def name_speakers(rows):
    """Pick the speaker->name mapping that puts the fewest seconds in a language that person doesn't speak."""
    dur = defaultdict(lambda: defaultdict(float))
    for r in rows:
        dur[r["spk"]][r["lang"]] += r["end"] - r["start"]
    spks = sorted(s for s in dur if s != "?")
    names = list(HINTS)
    best = min(itertools.permutations(names + [None] * max(0, len(spks) - len(names)), len(spks)),
               key=lambda perm: sum(sec for s, n in zip(spks, perm) if n for lang, sec in dur[s].items() if lang not in HINTS[n]))
    return {s: (n or s) for s, n in zip(spks, best)}, dur


def ts(t):
    return f"{int(t // 60):02d}:{int(t % 60):02d}"


def main():
    wav, diar_path, outdir = sys.argv[1:4]
    cpu = "--cpu" in sys.argv
    limit = float(sys.argv[sys.argv.index("--limit") + 1]) if "--limit" in sys.argv else None
    out, tmp = Path(outdir), Path(outdir) / "segs"
    tmp.mkdir(parents=True, exist_ok=True)
    timing = {}

    diar = diarization(diar_path)
    t = time.time(); segs = split_at_turns(vad_segments(wav, tmp), diar); timing["vad"] = time.time() - t
    if limit:
        segs = [s for s in segs if s[1] <= limit]
    paths = cut(wav, segs, tmp)

    t = time.time()
    subprocess.run(WHISPER + (["-ng"] if cpu else []) + [str(p) for p in paths], check=True, capture_output=True)
    timing["whisper"] = time.time() - t

    rows = []
    for (a, b), p in zip(segs, paths):
        j = json.loads(Path(str(p) + ".json").read_text())
        text = dedupe_loops(" ".join(x["text"].strip() for x in j["transcription"]).strip())
        if text:
            rows.append({"start": a, "end": b, "lang": j["result"]["language"], "spk": speaker_of(a, b, diar), "text": text})

    names, dur = name_speakers(rows)
    lines, prev = [], None
    for r in rows:
        key = (r["spk"], r["lang"])
        if key == prev:
            lines[-1] += " " + r["text"]
        else:
            lines.append(f"[{ts(r['start'])}] {names.get(r['spk'], r['spk'])} ({r['lang']}): {r['text']}")
        prev = key
    (out / "transcript.md").write_text("\n\n".join(lines) + "\n")
    (out / "rows.json").write_text(json.dumps(rows, ensure_ascii=False, indent=1))
    report = {"segments": len(segs), "audio_sec": sum(b - a for a, b in segs), "timing_sec": timing,
              "names": names, "lang_sec_by_speaker": {s: dict(v) for s, v in dur.items()}}
    (out / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=1))
    print(json.dumps(report, ensure_ascii=False, indent=1))


if __name__ == "__main__":
    main()
