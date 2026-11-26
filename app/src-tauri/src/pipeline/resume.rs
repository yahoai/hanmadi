//! Picking a stopped job back up: what it was asked to do, and what its work/ folder already holds.
use super::{is_live, launch, JobReq, LIVE};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::AppHandle;

/// Whether any job is queued or running here, so closing the window must not quit the app.
pub fn any_live() -> bool {
    !LIVE.lock().unwrap_or_else(|e| e.into_inner()).is_empty()
}

/// Status to show. Queued or running on disk but not in this process means the app quit mid-job.
pub fn status(doc: &Value) -> Value {
    match doc["status"].as_str() {
        Some("queued" | "running") if !is_live(doc["id"].as_str().unwrap_or_default()) => json!("stopped"),
        _ => doc["status"].clone(),
    }
}

/// Run a stopped or failed transcript again in its own folder; `fresh` throws work/ away first.
/// The request comes from the caller, else from what the job stored. Older transcripts stored none.
pub fn resume(app: Option<AppHandle>, transcripts: &Path, id: &str, fresh: bool, req: Option<JobReq>) -> Result<String, String> {
    let dir = transcripts.join(id);
    let doc: Value = fs::read_to_string(dir.join("transcript.json"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .ok_or("transcript not found")?;
    if is_live(id) {
        return Err("busy".into());
    }
    let req = req.or_else(|| serde_json::from_value(doc["req"].clone()).ok()).ok_or("needs-setup")?;
    if fresh {
        let _ = fs::remove_dir_all(dir.join("work"));
    }
    launch(app, transcripts, req, Some(id.into()))
}

/// A step's result kept in work/ as JSON, so a resumed job skips the step.
pub(super) fn cached<T: Serialize + DeserializeOwned>(path: &Path, make: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    if let Some(v) = fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()) {
        return Ok(v);
    }
    let v = make()?;
    let _ = fs::write(path, serde_json::to_string(&v).unwrap_or_default());
    Ok(v)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cache_round_trip() {
        let p = std::env::temp_dir().join(format!("hanmadi-cache-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&p);
        let made: Vec<(f64, f64, usize)> = super::cached(&p, || Ok(vec![(0.5, 2.25, 1)])).unwrap_or_default();
        let again: Vec<(f64, f64, usize)> = super::cached(&p, || Err("ran again".into())).unwrap_or_default();
        assert_eq!(made, again);
        assert_eq!(again, vec![(0.5, 2.25, 1)]);
        let _ = std::fs::remove_file(&p);
    }
}
