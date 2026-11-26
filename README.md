# Hanmadi

Hanmadi (한마디, "a word" in Korean) is a desktop app that turns a meeting recording into a transcript with timestamps, speakers, per-line language and a translation under each line. Everything runs on your own machine, so audio never leaves it and you don't need an account.

Built for meetings where people switch between Korean, Chinese, English and Japanese mid-conversation.

- Detects the language of each segment separately, so mixed-language meetings work
- Names speakers from the participant list you give it ("Mina speaks ko and zh, Wei speaks zh")
- Translates each line into any language with a local LLM, shown under the original
- Imports from a file or from macOS Voice Memos (iCloud)
- A glossary of names and product terms is fed to both transcription and translation

Status: early. Tested on macOS (Apple Silicon). Windows is planned but not tested yet.

## How it works

```
audio -> ffmpeg (16 kHz mono)
      -> sherpa-onnx VAD (silero) + speaker diarization (pyannote seg 3.0 + 3D-Speaker)
      -> cut at speaker changes, merge short pieces of the same speaker
      -> whisper.cpp large-v3-turbo per segment, language auto-detected
      -> llama.cpp + Qwen3 4B for translation and titles
```

The engines run as separate processes. The app is Tauri v2 (Rust) with a plain HTML/CSS/JS UI, no build step.

Rough speed for a 55 minute meeting on an M1 Pro: diarization 12 min on CPU, transcription 6 to 13 min with Metal, translation about 2 s per line.

## Build from source

You need Rust (pinned by `app/src-tauri/rust-toolchain.toml`), the Tauri CLI (`cargo install tauri-cli`), CMake and ffmpeg (`brew install ffmpeg`; the app looks for `/opt/homebrew/bin/ffmpeg` first).

Engines go in `tools/`, models in `models/`, both at the repo root:

```
tools/whisper.cpp/build/bin/whisper-cli
tools/llama.cpp/build/bin/llama-server
tools/sherpa/bin/sherpa-onnx-vad
tools/sherpa/bin/sherpa-onnx-offline-speaker-diarization
models/silero_vad.onnx
models/sherpa-onnx-pyannote-segmentation-3-0/model.onnx
models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx
models/ggml-large-v3-turbo-q5_0.bin
models/Qwen3-4B-Instruct-2507-Q4_K_M.gguf
```

```bash
# engines
git clone https://github.com/ggml-org/whisper.cpp tools/whisper.cpp
cmake -S tools/whisper.cpp -B tools/whisper.cpp/build && cmake --build tools/whisper.cpp/build -j --config Release
git clone https://github.com/ggml-org/llama.cpp tools/llama.cpp
cmake -S tools/llama.cpp -B tools/llama.cpp/build && cmake --build tools/llama.cpp/build -j --config Release
# sherpa-onnx: unpack a prebuilt release (e.g. sherpa-onnx-v2.0.3-osx-arm64-static-no-tts) into tools/sherpa
# https://github.com/k2-fsa/sherpa-onnx/releases

# models
S=https://github.com/k2-fsa/sherpa-onnx/releases/download
curl -L -o models/silero_vad.onnx $S/asr-models/silero_vad.onnx
curl -L $S/speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2 | tar xj -C models
curl -L -o models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx \
  $S/speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx
curl -L -o models/ggml-large-v3-turbo-q5_0.bin \
  https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin
curl -L -o models/Qwen3-4B-Instruct-2507-Q4_K_M.gguf \
  https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf
```

Run:

```bash
cd app/src-tauri
cargo tauri dev
cargo test        # pipeline unit tests
```

For now the app reads engines and models from the source checkout, so a packaged build is not ready yet.

Transcripts are saved to `~/Library/Application Support/io.github.yahoai.hanmadi/transcripts/<id>/`.

## Third-party licenses

Hanmadi is MIT. The pieces it runs keep their own licenses: whisper.cpp and llama.cpp (MIT), sherpa-onnx (Apache-2.0), Whisper weights (MIT), Silero VAD (MIT), pyannote segmentation 3.0 (MIT), 3D-Speaker (Apache-2.0), Qwen3 (Apache-2.0). Homebrew's ffmpeg is built with GPL components. That is fine for local use, but if you redistribute a bundle with ffmpeg inside, use an LGPL build.

## License

[MIT](LICENSE)
