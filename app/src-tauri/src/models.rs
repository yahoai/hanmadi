//! First-run download of the models (about 3 GB) into app data. Each file is fetched to `<name>.part`,
//! resumed from there when a download breaks, and only gets its real name once its size checks out.
use crate::engines::{model, models_dir};
use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

struct Model {
    /// Path under models/ the pipeline reads.
    file: &'static str,
    /// Full URL, or a path under the sherpa-onnx release downloads.
    url: &'static str,
    /// Bytes of what `url` serves.
    size: u64,
    /// The download is a .tar.bz2 that unpacks to `file`.
    archive: bool,
}

const S: &str = "https://github.com/k2-fsa/sherpa-onnx/releases/download";

const MODELS: [Model; 5] = [
    Model { file: "silero_vad.onnx", url: "asr-models/silero_vad.onnx", size: 643_854, archive: false },
    Model {
        file: "sherpa-onnx-pyannote-segmentation-3-0/model.onnx",
        url: "speaker-segmentation-models/sherpa-onnx-pyannote-segmentation-3-0.tar.bz2",
        size: 6_958_444,
        archive: true,
    },
    Model {
        file: "3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
        url: "speaker-recongition-models/3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx",
        size: 39_593_762,
        archive: false,
    },
    Model {
        file: "ggml-large-v3-turbo-q5_0.bin",
        url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3-turbo-q5_0.bin",
        size: 574_041_195,
        archive: false,
    },
    Model {
        file: "Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        url: "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/Qwen3-4B-Instruct-2507-Q4_K_M.gguf",
        size: 2_497_281_120,
        archive: false,
    },
];

/// sherpa-onnx models are given relative to its release downloads.
fn url(m: &Model) -> String {
    if m.url.starts_with("https://") { m.url.into() } else { format!("{S}/{}", m.url) }
}

static BUSY: AtomicBool = AtomicBool::new(false);

fn missing() -> Vec<&'static Model> {
    MODELS.iter().filter(|m| !model(m.file).is_file()).collect()
}

pub fn ready() -> bool {
    missing().is_empty()
}

#[tauri::command]
pub fn models_status() -> Value {
    let need = missing();
    json!({
        "ready": need.is_empty(),
        "bytes": need.iter().map(|m| m.size).sum::<u64>(),
        "downloading": BUSY.load(Ordering::SeqCst),
    })
}

/// Starts the download in the background. Progress comes as `models-progress` events
/// ({have, total} while going, {done: true} or {error} at the end).
#[tauri::command]
pub fn download_models(app: AppHandle) {
    if BUSY.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(move || {
        let res = download_all(&app);
        BUSY.store(false, Ordering::SeqCst);
        let _ = app.emit("models-progress", match res {
            Ok(()) => json!({"done": true}),
            Err(e) => json!({"error": e}),
        });
    });
}

fn download_all(app: &AppHandle) -> Result<(), String> {
    let dir = models_dir();
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let todo = missing();
    let total: u64 = todo.iter().map(|m| m.size).sum();
    let mut before = 0;
    let mut last = Instant::now();
    for m in todo {
        let name = url(m).rsplit('/').next().unwrap_or(m.file).to_string();
        let part = dir.join(format!("{name}.part"));
        fetch(&url(m), &part, m.size, |have| {
            if last.elapsed() > Duration::from_millis(250) {
                last = Instant::now();
                let _ = app.emit("models-progress", json!({"have": before + have, "total": total}));
            }
        })?;
        let dest = dir.join(m.file);
        if m.archive {
            let out = std::process::Command::new("/usr/bin/tar").arg("-xjf").arg(&part).arg("-C").arg(&dir).output().map_err(|e| e.to_string())?;
            if !out.status.success() || !dest.is_file() {
                let _ = fs::remove_file(&part);
                return Err(format!("could not unpack {name}"));
            }
            let _ = fs::remove_file(&part);
        } else {
            fs::rename(&part, &dest).map_err(|e| e.to_string())?;
        }
        before += m.size;
    }
    Ok(())
}

/// Download `url` into `part` until it holds `size` bytes, continuing what an earlier try left there.
fn fetch(url: &str, part: &Path, size: u64, mut tick: impl FnMut(u64)) -> Result<(), String> {
    let mut have = fs::metadata(part).map(|m| m.len()).unwrap_or(0);
    if have > size {
        let _ = fs::remove_file(part);
        have = 0;
    }
    if have < size {
        let agent = ureq::AgentBuilder::new().timeout_connect(Duration::from_secs(30)).timeout_read(Duration::from_secs(60)).build();
        let mut req = agent.get(url);
        if have > 0 {
            req = req.set("Range", &format!("bytes={have}-"));
        }
        let res = req.call().map_err(|e| format!("download failed: {e}"))?;
        // a server that ignores Range sends the whole file again
        if res.status() != 206 {
            have = 0;
        }
        let mut file = fs::OpenOptions::new().create(true).write(true).append(have > 0).truncate(have == 0).open(part).map_err(|e| e.to_string())?;
        let mut body = res.into_reader();
        let mut buf = vec![0u8; 1 << 20];
        loop {
            let n = body.read(&mut buf).map_err(|e| format!("download failed: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n]).map_err(|e| e.to_string())?;
            have += n as u64;
            tick(have);
        }
    }
    if have != size {
        return Err("download incomplete, try again".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_are_absolute() {
        for m in &MODELS {
            assert!(url(m).starts_with("https://"), "{}", m.file);
        }
    }

    /// Network: downloads the smallest model, cuts it short, and checks the resume finishes it.
    #[test]
    #[ignore]
    fn fetch_resumes() {
        let m = &MODELS[0];
        let part = std::env::temp_dir().join("hanmadi-fetch-test.part");
        let _ = fs::remove_file(&part);
        fetch(&url(m), &part, m.size, |_| {}).unwrap();
        let f = fs::OpenOptions::new().write(true).open(&part).unwrap();
        f.set_len(m.size / 3).unwrap();
        let mut first = None;
        fetch(&url(m), &part, m.size, |have| { first.get_or_insert(have); }).unwrap();
        assert_eq!(fs::metadata(&part).unwrap().len(), m.size);
        eprintln!("resumed at {:?} of {}", first, m.size);
        let _ = fs::remove_file(&part);
    }
}
