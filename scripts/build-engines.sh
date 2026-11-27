#!/bin/bash
# Builds the engine binaries a packaged Hanmadi carries in Contents/Resources/engines.
# All arm64 (Apple Silicon), statically linked so each one is a single self-contained file:
#   whisper-cli   whisper.cpp, Metal shaders embedded
#   llama-server  llama.cpp, Metal shaders embedded
#   sherpa-onnx-vad, sherpa-onnx-offline-speaker-diarization   sherpa-onnx prebuilt static release
#   ffmpeg, ffprobe   LGPL build (no GPL parts), so it may ship inside the app
#
# Usage: scripts/build-engines.sh          (output: build/engines/, sources in build/src/)
# Works on an Intel Mac too (cross-compiles), it just can't run the results.
set -euo pipefail

export PATH="$PATH:/usr/local/bin:/opt/homebrew/bin"

WHISPER_TAG=v1.9.6
LLAMA_TAG=v1.1.0
SHERPA_VER=2.0.3
FFMPEG_TAG=n9.0.2
# ggml's own macOS builds target 13.3; keep tauri.conf.json minimumSystemVersion in step
MACOS_MIN=13.3

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/build/src"
OUT="$ROOT/build/engines"
JOBS="$(sysctl -n hw.ncpu)"
mkdir -p "$SRC" "$OUT"

log() { echo "[engines] $*"; }
die() { echo "[engines] ERROR: $*" >&2; exit 1; }
command -v cmake >/dev/null || die "cmake missing (brew install cmake)"

fetch() { # repo tag dir
  if [ ! -d "$3/.git" ]; then
    git clone -q --depth 1 --branch "$2" "https://github.com/$1" "$3"
  fi
  [ "$(git -C "$3" describe --tags --exact-match 2>/dev/null)" = "$2" ] || die "$3 is not at $2 (delete it to refetch)"
}

CMAKE_COMMON=(
  -DCMAKE_BUILD_TYPE=Release
  -DCMAKE_OSX_ARCHITECTURES=arm64
  -DCMAKE_OSX_DEPLOYMENT_TARGET="$MACOS_MIN"
  -DBUILD_SHARED_LIBS=OFF
  -DGGML_NATIVE=OFF
  -DGGML_METAL=ON
  -DGGML_METAL_EMBED_LIBRARY=ON
)

log "whisper.cpp $WHISPER_TAG"
fetch ggml-org/whisper.cpp "$WHISPER_TAG" "$SRC/whisper.cpp"
cmake -S "$SRC/whisper.cpp" -B "$SRC/whisper.cpp/build-arm64" "${CMAKE_COMMON[@]}" \
  -DWHISPER_BUILD_TESTS=OFF -DWHISPER_BUILD_SERVER=OFF >/dev/null
cmake --build "$SRC/whisper.cpp/build-arm64" -j "$JOBS" --config Release --target whisper-cli
cp "$SRC/whisper.cpp/build-arm64/bin/whisper-cli" "$OUT/"

log "llama.cpp $LLAMA_TAG"
fetch ggml-org/llama.cpp "$LLAMA_TAG" "$SRC/llama.cpp"
cmake -S "$SRC/llama.cpp" -B "$SRC/llama.cpp/build-arm64" "${CMAKE_COMMON[@]}" \
  -DLLAMA_CURL=OFF -DLLAMA_OPENSSL=OFF -DLLAMA_BUILD_TESTS=OFF -DLLAMA_BUILD_EXAMPLES=OFF >/dev/null
cmake --build "$SRC/llama.cpp/build-arm64" -j "$JOBS" --config Release --target llama-server
cp "$SRC/llama.cpp/build-arm64/bin/llama-server" "$OUT/"

log "sherpa-onnx $SHERPA_VER"
SHERPA_NAME="sherpa-onnx-v$SHERPA_VER-osx-arm64-static-no-tts"
if [ ! -d "$SRC/$SHERPA_NAME" ]; then
  curl -fsSL "https://github.com/k2-fsa/sherpa-onnx/releases/download/v$SHERPA_VER/$SHERPA_NAME.tar.bz2" | tar xj -C "$SRC"
fi
for b in sherpa-onnx-vad sherpa-onnx-offline-speaker-diarization; do
  cp "$SRC/$SHERPA_NAME/bin/$b" "$OUT/"
done

log "ffmpeg $FFMPEG_TAG (LGPL)"
fetch FFmpeg/FFmpeg "$FFMPEG_TAG" "$SRC/ffmpeg"
FF_BUILD="$SRC/ffmpeg/build-arm64"
if [ ! -x "$FF_BUILD/ffmpeg" ]; then
  mkdir -p "$FF_BUILD"
  # --disable-autodetect keeps Homebrew libraries (and their licenses) out; only Apple frameworks are linked
  (cd "$FF_BUILD" && ../configure \
    --enable-cross-compile --arch=arm64 --target-os=darwin \
    --cc="clang -arch arm64 -mmacosx-version-min=$MACOS_MIN" \
    --extra-ldflags="-arch arm64 -mmacosx-version-min=$MACOS_MIN" \
    --pkg-config=/usr/bin/false \
    --disable-autodetect --enable-avfoundation --enable-audiotoolbox --enable-zlib \
    --disable-gpl --disable-nonfree --disable-network \
    --enable-static --disable-shared --disable-doc --disable-debug --disable-ffplay)
  make -C "$FF_BUILD" -j "$JOBS" ffmpeg ffprobe
fi
cp "$FF_BUILD/ffmpeg" "$FF_BUILD/ffprobe" "$OUT/"
grep -q '^#define CONFIG_GPL 0' "$FF_BUILD/config.h" || die "ffmpeg came out GPL"
grep -q '^#define CONFIG_AVFOUNDATION_INDEV 1' "$FF_BUILD/config_components.h" || die "ffmpeg has no microphone input (avfoundation)"

log "checking"
for f in "$OUT"/*; do
  [ "$(lipo -archs "$f")" = arm64 ] || die "$(basename "$f") is not arm64"
  # only system libraries may be linked: anything else would be missing on another Mac
  if otool -L "$f" | tail -n +2 | awk '{print $1}' | grep -v -E '^(/usr/lib/|/System/Library/)'; then
    die "$(basename "$f") links a non-system library"
  fi
done
ls -lh "$OUT"
log "done → $OUT"
