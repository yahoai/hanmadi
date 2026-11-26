//! Recording a meeting from the default microphone into app_data/recordings/. ffmpeg writes the m4a and
//! runs silencedetect, so the UI can show whether sound is coming in. The recording ends when the user
//! stops it, after 5 quiet minutes, or when the computer sleeps. These files are the originals: deleting
//! a transcript never touches them.
use crate::pipeline::{exec, ffmpeg, probe};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStderr, Command, Stdio};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager};

/// Seconds of silence that end a recording. silencedetect itself reports after 2 quiet seconds (for the indicator).
const QUIET_STOP: f64 = 300.0;
const QUIET_REPORT: f64 = 2.0;

struct Rec {
    child: Child,
    path: String,
    id: u128,
    /// Media time the current silence began, and when we heard about it.
    quiet: Option<(f64, Instant)>,
}

static REC: Mutex<Option<Rec>> = Mutex::new(None);

fn rec() -> MutexGuard<'static, Option<Rec>> {
    REC.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn active() -> bool {
    rec().is_some()
}

#[cfg(target_os = "macos")]
fn input() -> Result<Vec<String>, String> {
    Ok(["-f", "avfoundation", "-i", ":default"].map(String::from).to_vec())
}

#[cfg(not(target_os = "macos"))]
fn input() -> Result<Vec<String>, String> {
    // ponytail: first DirectShow audio device; add a picker when people have several microphones
    let out = Command::new(ffmpeg())
        .args(["-hide_banner", "-list_devices", "true", "-f", "dshow", "-i", "dummy"])
        .output()
        .map_err(|e| e.to_string())?;
    let log = String::from_utf8_lossy(&out.stderr);
    let name = log.lines().filter(|l| l.contains("(audio)")).find_map(|l| l.split('"').nth(1)).ok_or("no microphone found")?;
    Ok(vec!["-f".into(), "dshow".into(), "-i".into(), format!("audio={name}")])
}

/// `stamp` is the local time from the UI ("2026-11-26 15-04-05"); Rust has no local clock without a date library.
#[tauri::command]
pub async fn start_recording(app: AppHandle, stamp: String) -> Result<String, String> {
    if active() {
        return Err("already recording".into());
    }
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?.join("recordings");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let name: String = stamp.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == ' ').collect();
    let path = dir.join(format!("{}.m4a", if name.trim().is_empty() { "recording" } else { name.trim() })).to_string_lossy().to_string();
    let mut child = Command::new(ffmpeg())
        .args(["-hide_banner", "-nostats", "-n"])
        .args(input()?)
        .args(["-af", "silencedetect=noise=-45dB:d=2", "-ac", "1", "-c:a", "aac", "-b:a", "80k", &path])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("ffmpeg: {e}"))?;
    // no device, a denied microphone or an existing file ends ffmpeg right away
    std::thread::sleep(Duration::from_millis(800));
    if child.try_wait().map_err(|e| e.to_string())?.is_some() {
        let mut log = String::new();
        let _ = child.stderr.take().map(|mut s| s.read_to_string(&mut log));
        return Err(log.lines().last().unwrap_or("could not record").to_string());
    }
    let id = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default().as_nanos();
    let err = child.stderr.take();
    *rec() = Some(Rec { child, path: path.clone(), id, quiet: None });
    let a = app.clone();
    std::thread::spawn(move || listen(a, id, err));
    std::thread::spawn(move || watch(app, id));
    Ok(path)
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> Result<String, String> {
    finish(&app, None, "user", None)
}

/// silencedetect lines: `[Parsed_silencedetect_0 @ 0x..] silence_start: 12.3` and `silence_end: 20.1 | silence_duration: 7.8`.
fn listen(app: AppHandle, id: u128, err: Option<ChildStderr>) {
    for line in err.into_iter().flat_map(|e| BufReader::new(e).lines().map_while(Result::ok)) {
        let at = |k: &str| line.split(k).nth(1).and_then(|s| s.split_whitespace().next()?.parse::<f64>().ok());
        let quiet = match (at("silence_start: "), at("silence_end: ")) {
            (Some(t), _) => Some((t, Instant::now())),
            (_, Some(_)) => None,
            _ => continue,
        };
        if let Some(r) = rec().as_mut().filter(|r| r.id == id) {
            r.quiet = quiet;
        }
        let _ = app.emit("rec-state", json!({"quiet": quiet.is_some()}));
    }
    // ffmpeg ended by itself (microphone unplugged, crash); no-op when we stopped it
    let _ = finish(&app, Some(id), "error", None);
}

/// Every 2s: stop after 5 quiet minutes, or when the wall clock jumped (threads don't run while the computer sleeps).
fn watch(app: AppHandle, id: u128) {
    let mut last = SystemTime::now();
    loop {
        std::thread::sleep(Duration::from_secs(2));
        let now = SystemTime::now();
        let slept = now.duration_since(last).unwrap_or_default() > Duration::from_secs(30);
        last = now;
        let quiet = match rec().as_ref() {
            Some(r) if r.id == id => r.quiet,
            _ => return,
        };
        if slept {
            let _ = finish(&app, Some(id), "sleep", None);
            return;
        }
        if let Some((t, _)) = quiet.filter(|(_, heard)| heard.elapsed().as_secs_f64() + QUIET_REPORT >= QUIET_STOP) {
            let _ = finish(&app, Some(id), "silence", Some(t));
            return;
        }
    }
}

/// Ends the recording (only if `id` is still the current one), letting ffmpeg finalize the file,
/// and emits `rec-stopped` {path, reason, duration}.
fn finish(app: &AppHandle, id: Option<u128>, reason: &str, cut: Option<f64>) -> Result<String, String> {
    let mut r = {
        let mut g = rec();
        if id.is_some() && g.as_ref().map(|r| r.id) != id {
            return Err("not recording".into());
        }
        g.take().ok_or("not recording")?
    };
    // "q" makes ffmpeg write the index; kill only if it hangs
    let _ = r.child.stdin.as_mut().map(|s| s.write_all(b"q"));
    let end = Instant::now() + Duration::from_secs(10);
    while matches!(r.child.try_wait(), Ok(None)) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(100));
    }
    let _ = r.child.kill();
    let _ = r.child.wait();
    if let Some(t) = cut {
        // drop the trailing silence, keeping a second of it; remux only
        let tmp = format!("{}.trim.m4a", r.path);
        let len = format!("{:.2}", t + 1.0);
        if exec(Command::new(ffmpeg()).args(["-v", "error", "-y", "-i", &r.path, "-t", &len, "-c", "copy", &tmp])).is_ok() {
            let _ = std::fs::rename(&tmp, &r.path);
        }
    }
    let duration = probe(&r.path)["duration"].clone();
    let _ = app.emit("rec-stopped", json!({"path": r.path, "reason": reason, "duration": duration}));
    Ok(r.path)
}
