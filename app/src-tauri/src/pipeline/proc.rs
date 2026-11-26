//! Running the engine binaries, and reading facts about a source file.
use super::timing::iso;
use serde_json::{json, Value};
use std::path::Path;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

pub fn ffmpeg() -> String {
    // ponytail: Homebrew ffmpeg on the dev Mac, PATH elsewhere; bundle a static ffmpeg for distribution
    let brew = "/opt/homebrew/bin/ffmpeg";
    if Path::new(brew).exists() { brew.into() } else { "ffmpeg".into() }
}

fn ffprobe() -> String {
    ffmpeg().replace("ffmpeg", "ffprobe")
}

pub fn exec(cmd: &mut Command) -> Result<String, String> {
    let out = cmd.output().map_err(|e| format!("{:?}: {e}", cmd.get_program()))?;
    let all = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        let tail: Vec<&str> = all.lines().rev().take(3).collect();
        return Err(format!("{:?} failed: {}", cmd.get_program(), tail.join(" | ")));
    }
    Ok(all)
}

/// Poll a child process, reporting elapsed seconds to `tick`, and return its stdout.
pub fn wait_with<T>(ctx: &mut T, mut child: Child, mut tick: impl FnMut(&mut T, f64)) -> Result<String, String> {
    use std::io::Read;
    let start = Instant::now();
    let mut stdout = child.stdout.take();
    let reader = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(o) = stdout.as_mut() {
            let _ = o.read_to_string(&mut s);
        }
        s
    });
    loop {
        if let Some(st) = child.try_wait().map_err(|e| e.to_string())? {
            if !st.success() {
                return Err(format!("engine exited with {st}"));
            }
            return Ok(reader.join().unwrap_or_default());
        }
        tick(ctx, start.elapsed().as_secs_f64());
        std::thread::sleep(Duration::from_millis(700));
    }
}

/// When the recording was made and how long it is. The date comes from the file's own
/// creation_time tag (phones and recorders write it), else the file's creation date on disk.
pub fn probe(path: &str) -> Value {
    let out = Command::new(ffprobe())
        .args(["-v", "error", "-show_entries", "format=duration:format_tags=creation_time", "-of", "json", path])
        .output()
        .ok()
        .and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok())
        .unwrap_or_default();
    let fmt = &out["format"];
    let tagged = fmt["tags"]["creation_time"].as_str().filter(|s| !s.starts_with("1970")).map(String::from);
    let on_disk = std::fs::metadata(path).ok().and_then(|m| m.created().or_else(|_| m.modified()).ok()).map(iso);
    let duration = fmt["duration"].as_str().and_then(|d| d.parse::<f64>().ok());
    json!({"recorded": tagged.or(on_disk), "duration": duration})
}

/// A copy WebKit can play: the first audio track (AAC in Voice Memos .qta) remuxed without re-encoding.
/// Named per source so the webview never serves a stale cached file. The source is only read.
pub fn preview(path: &str) -> Result<String, String> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut h);
    let out = std::env::temp_dir().join(format!("hanmadi-preview-{:x}.m4a", h.finish()));
    exec(Command::new(ffmpeg()).args(["-v", "error", "-y", "-i", path, "-map", "0:a:0", "-c", "copy"]).arg(&out))?;
    Ok(out.to_string_lossy().into())
}
