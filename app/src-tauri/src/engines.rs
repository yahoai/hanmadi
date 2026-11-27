//! Where the engine binaries and the models are. A packaged app carries the engines in
//! Contents/Resources/engines (scripts/build-engines.sh) and downloads the models into app data on
//! first run (models.rs). A dev build falls back to tools/ and models/ in the source checkout.
use std::path::{Path, PathBuf};

fn checkout() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The app's data folder, also reachable from the command line where there is no Tauri app.
// ponytail: macOS path only; the GUI resolves the same folder through Tauri
pub fn app_data() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join("Library/Application Support/io.github.yahoai.hanmadi")
}

fn bundled(name: &str) -> Option<PathBuf> {
    // Contents/MacOS/hanmadi -> Contents/Resources/engines/<name>
    let p = std::env::current_exe().ok()?.parent()?.join("../Resources/engines").join(name);
    p.is_file().then_some(p)
}

/// whisper-cli, llama-server, sherpa-onnx-vad, sherpa-onnx-offline-speaker-diarization, ffmpeg or ffprobe.
pub fn engine(name: &str) -> PathBuf {
    if let Some(p) = bundled(name) {
        return p;
    }
    match name {
        "whisper-cli" => checkout().join("tools/whisper.cpp/build/bin/whisper-cli"),
        "llama-server" => checkout().join("tools/llama.cpp/build/bin/llama-server"),
        "ffmpeg" | "ffprobe" => ["/opt/homebrew/bin", "/usr/local/bin"]
            .iter()
            .map(|d| Path::new(d).join(name))
            .find(|p| p.exists())
            .unwrap_or_else(|| name.into()),
        _ => checkout().join("tools/sherpa/bin").join(name),
    }
}

pub fn models_dir() -> PathBuf {
    app_data().join("models")
}

/// A model file by its path under models/: the downloaded copy, else the dev checkout's.
pub fn model(rel: &str) -> PathBuf {
    let p = models_dir().join(rel);
    let dev = checkout().join("models").join(rel);
    if !p.exists() && dev.exists() { dev } else { p }
}
