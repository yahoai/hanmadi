import sys, json
import numpy as np, mlx_whisper
from mlx_whisper.audio import load_audio

# gap ranges (sec) where the Korean-only Voice Memos transcript broke
RANGES = [(0, 610), (1040, 1225), (1350, 1590), (1945, 2085), (2475, 2640)]
M = "mlx-community/whisper-large-v3-turbo"
audio = load_audio(sys.argv[1])
out = []
for a, b in RANGES:
    # 60s chunks so language is detected per chunk (meeting switches languages)
    for s in range(a, b, 60):
        e = min(s + 60, b)
        r = mlx_whisper.transcribe(audio[s * 16000:e * 16000], path_or_hf_repo=M,
                                   condition_on_previous_text=False)
        t = " ".join(x["text"].strip() for x in r["segments"])
        line = f"[{s//60:02d}:{s%60:02d}-{e//60:02d}:{e%60:02d}] ({r['language']}) {t}"
        print(line, flush=True)
        out.append(line)
open("/tmp/rec22/gaps.txt", "w").write("\n".join(out))
