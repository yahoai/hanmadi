#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

mod pipeline;
mod record;

fn transcripts_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let d = app.path().app_data_dir().map_err(|e| e.to_string())?.join("transcripts");
    fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn read_doc(dir: &PathBuf) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(dir.join("transcript.json")).ok()?).ok()
}

#[tauri::command]
fn list_transcripts(app: AppHandle) -> Result<Vec<Value>, String> {
    let mut out: Vec<Value> = fs::read_dir(transcripts_dir(&app)?)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| read_doc(&e.path()))
        .map(|d| json!({
            "id": d["id"], "title": d["title"], "recorded": d["recorded"], "added": d["added"], "duration": d["duration"],
            "languages": d["languages"], "status": pipeline::status(&d), "progress": d["progress"], "source": d["source"],
            "vocab": d["vocab"], "terms": d["terms"],
            "names": d["speakers"].as_array().into_iter().flatten().filter_map(|s| s["name"].as_str()).filter(|n| !n.trim().is_empty()).collect::<Vec<_>>(),
        }))
        .collect();
    // newest addition first; the recording date is shown, but people look for what they just added
    let key = |d: &Value| d["added"].as_str().or(d["recorded"].as_str()).unwrap_or("").to_string();
    out.sort_by_key(|d| std::cmp::Reverse(key(d)));
    Ok(out)
}

#[tauri::command]
fn get_transcript(app: AppHandle, id: String) -> Result<Value, String> {
    let dir = transcripts_dir(&app)?.join(&id);
    let mut d = read_doc(&dir).ok_or("transcript not found")?;
    d["audioPath"] = json!(dir.join("audio.m4a"));
    d["status"] = pipeline::status(&d);
    Ok(d)
}

#[tauri::command]
fn save_transcript(app: AppHandle, doc: Value) -> Result<(), String> {
    let id = doc["id"].as_str().ok_or("missing id")?;
    let mut doc = doc.clone();
    if let Some(o) = doc.as_object_mut() {
        o.remove("audioPath");
    }
    let path = transcripts_dir(&app)?.join(id).join("transcript.json");
    fs::write(path, serde_json::to_string(&doc).unwrap_or_default()).map_err(|e| e.to_string())
}

/// Ids are folder names we made (millis, or a dated slug). Anything else could step outside transcripts/.
fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Removes the transcript folder (transcript.json, audio.m4a, work/). The original recording is never touched.
#[tauri::command]
fn delete_transcript(app: AppHandle, id: String) -> Result<(), String> {
    if !safe_id(&id) {
        return Err("invalid id".into());
    }
    if pipeline::is_live(&id) {
        return Err("busy".into());
    }
    let dir = transcripts_dir(&app)?.join(&id);
    if !dir.is_dir() {
        return Err("transcript not found".into());
    }
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

/// macOS Voice Memos synced via iCloud. Reads the app's own SQLite index with the system sqlite3.
#[tauri::command]
fn list_voice_memos() -> Result<Value, String> {
    let home = std::env::var("HOME").map_err(|e| e.to_string())?;
    let dir = format!("{home}/Library/Group Containers/group.com.apple.VoiceMemos.shared/Recordings");
    let sql = "select ZENCRYPTEDTITLE as title, ZDATE+978307200 as ts, ZDURATION as duration, ZPATH as file \
               from ZCLOUDRECORDING where ZDURATION > 60 order by ZDATE desc limit 100";
    let out = std::process::Command::new("/usr/bin/sqlite3")
        .args(["-readonly", "-json", &format!("{dir}/CloudRecordings.db"), sql])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into());
    }
    let mut rows: Value = serde_json::from_slice(&out.stdout).unwrap_or(json!([]));
    for r in rows.as_array_mut().into_iter().flatten() {
        r["path"] = json!(format!("{dir}/{}", r["file"].as_str().unwrap_or_default()));
    }
    Ok(rows)
}

fn clock(s: f64) -> String {
    format!("{:02}:{:02}", (s / 60.0) as u64, (s % 60.0) as u64)
}

#[tauri::command]
fn export_markdown(app: AppHandle, id: String, mode: String) -> Result<String, String> {
    let d = read_doc(&transcripts_dir(&app)?.join(&id)).ok_or("transcript not found")?;
    let names: Vec<String> = d["speakers"].as_array().into_iter().flatten()
        .enumerate()
        .map(|(i, s)| s["name"].as_str().filter(|n| !n.is_empty()).map(String::from).unwrap_or(format!("Speaker {}", i + 1)))
        .collect();
    let mut md = format!("# {}\n\n", d["title"].as_str().unwrap_or("Transcript"));
    for t in d["turns"].as_array().into_iter().flatten() {
        let who = names.get(t["s"].as_u64().unwrap_or(0) as usize).cloned().unwrap_or_default();
        let (orig, tr) = (t["x"].as_str().unwrap_or(""), t["tr"].as_str());
        let body = match (mode.as_str(), tr) {
            ("tr", Some(tr)) => tr.to_string(),
            ("both", Some(tr)) => format!("{orig}\n  > {tr}"),
            _ => orig.to_string(),
        };
        md += &format!("[{}] {} ({}): {}\n\n", clock(t["t"].as_f64().unwrap_or(0.0)), who, t["l"].as_str().unwrap_or(""), body);
    }
    let file = app.path().download_dir().map_err(|e| e.to_string())?
        .join(format!("{}.md", d["title"].as_str().unwrap_or(&id).replace('/', "-")));
    fs::write(&file, md).map_err(|e| e.to_string())?;
    Ok(file.to_string_lossy().into())
}

/// Expected seconds for transcription and translation, from this machine's measured speed.
#[tauri::command]
fn estimate(app: AppHandle, duration: f64, translate: bool) -> Result<Value, String> {
    Ok(pipeline::estimate(&transcripts_dir(&app)?, duration, translate))
}

/// Recording date and length of a file the user dropped in.
#[tauri::command]
fn probe_source(path: String) -> Value {
    pipeline::probe(&path)
}

/// Playable temp copy of a recording for the preview player (Voice Memos .qta). Async so ffmpeg runs off the UI thread.
#[tauri::command]
async fn preview_audio(path: String) -> Result<String, String> {
    pipeline::preview(&path)
}

/// Let the local model title a finished transcript. Takes a few seconds, so it runs off the UI thread.
#[tauri::command]
async fn retitle(app: AppHandle, id: String, lang: String) -> Result<String, String> {
    let dir = transcripts_dir(&app)?.join(&id);
    tauri::async_runtime::spawn_blocking(move || pipeline::retitle(&dir, &lang)).await.map_err(|e| e.to_string())?
}

/// Vocabulary suggestions for a transcript made before terms were collected. Stored in transcript.json as `terms`.
#[tauri::command]
async fn suggest_terms(app: AppHandle, id: String) -> Result<Vec<String>, String> {
    if !safe_id(&id) {
        return Err("invalid id".into());
    }
    let dir = transcripts_dir(&app)?.join(&id);
    tauri::async_runtime::spawn_blocking(move || pipeline::terms(&dir)).await.map_err(|e| e.to_string())?
}

#[tauri::command]
fn start_job(app: AppHandle, req: pipeline::JobReq) -> Result<String, String> {
    let dir = transcripts_dir(&app)?;
    pipeline::start(app, dir, req)
}

/// Run a stopped or failed transcript again: reuse its work/ (`fresh` false) or start over. Err "needs-setup" when no request is stored.
#[tauri::command]
fn resume_job(app: AppHandle, id: String, fresh: bool, req: Option<pipeline::JobReq>) -> Result<String, String> {
    if !safe_id(&id) {
        return Err("invalid id".into());
    }
    pipeline::resume(Some(app.clone()), &transcripts_dir(&app)?, &id, fresh, req)
}

/// Dev/batch entry: `hanmadi --retranslate <id> [--to ko]`, `hanmadi --retitle <id> [--lang ko]`, `hanmadi --terms <id>`, `hanmadi --resume <id> [--fresh]`, or `hanmadi --transcribe <file> [--speakers "Mina:ko,zh;Wei:zh"] [--vocab "Nimbus,Orion"] [--to ko] [--title t]`
fn cli(args: &[String]) -> Option<Result<String, String>> {
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    // ponytail: macOS app-data path only; the GUI resolves it through Tauri
    let dir = PathBuf::from(std::env::var("HOME").ok()?).join("Library/Application Support/io.github.yahoai.hanmadi/transcripts");
    if let Some(id) = opt("--retranslate") {
        let to = opt("--to").unwrap_or_else(|| "ko".into());
        return Some(pipeline::retranslate(&dir.join(&id), &to).map(|_| id));
    }
    if let Some(id) = opt("--retitle") {
        return Some(pipeline::retitle(&dir.join(&id), &opt("--lang").unwrap_or_else(|| "ko".into())));
    }
    if let Some(id) = opt("--terms") {
        return Some(pipeline::terms(&dir.join(&id)).map(|t| t.join("\n")));
    }
    if let Some(id) = opt("--resume").filter(|id| safe_id(id)) {
        return Some(pipeline::resume(None, &dir, &id, args.iter().any(|a| a == "--fresh"), None));
    }
    let at = args.iter().position(|a| a == "--transcribe")?;
    let path = args.get(at + 1)?.clone();
    let speakers = opt("--speakers").unwrap_or_default().split(';').filter(|s| !s.is_empty()).map(|s| {
        let (name, langs) = s.split_once(':').unwrap_or((s, ""));
        pipeline::Speaker { name: name.into(), langs: langs.split(',').filter(|l| !l.is_empty()).map(String::from).collect() }
    }).collect();
    let title = opt("--title").unwrap_or_else(|| path.rsplit('/').next().unwrap_or(&path).to_string());
    let probed = pipeline::probe(&path)["recorded"].as_str().unwrap_or_default().to_string();
    let req = pipeline::JobReq {
        source: pipeline::Source { kind: "file".into(), recorded: opt("--recorded").unwrap_or(probed), path },
        title,
        speakers,
        vocab: opt("--vocab").unwrap_or_default().split(',').filter(|s| !s.is_empty()).map(String::from).collect(),
        translate_to: opt("--to"),
        auto_title: opt("--title").is_none(),
        title_lang: opt("--lang"),
    };
    Some(pipeline::run_blocking(dir, req))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(res) = cli(&args) {
        match res {
            Ok(id) => println!("done {id}"),
            Err(e) => {
                eprintln!("failed: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_window_state::Builder::default().build())
        .invoke_handler(tauri::generate_handler![
            list_transcripts, get_transcript, save_transcript, delete_transcript, list_voice_memos, export_markdown, start_job, resume_job, estimate, probe_source, retitle, preview_audio, suggest_terms,
            record::start_recording, record::stop_recording
        ])
        // closing the window must not kill a job: hide it (macOS, the dock brings it back) or minimize it instead
        .on_window_event(|w, e| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                if pipeline::any_live() || record::active() {
                    api.prevent_close();
                    let _ = if cfg!(target_os = "macos") { w.hide() } else { w.minimize() };
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to run app")
        .run(|app, e| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = e {
                for w in app.webview_windows().values() {
                    let _ = (w.show(), w.set_focus());
                }
            }
            let _ = (app, e);
        });
}

#[cfg(test)]
mod tests {
    #[test]
    fn safe_id_rejects_paths() {
        assert!(super::safe_id("1795668724448") && super::safe_id("2026-11-25-acme"));
        for bad in ["", "..", "../x", "a/b", "a\\b", "rates.json", "/tmp"] {
            assert!(!super::safe_id(bad), "{bad}");
        }
    }
}
