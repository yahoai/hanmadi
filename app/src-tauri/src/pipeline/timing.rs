//! How long each step takes on this machine: estimates before and during a job, measurements after it.
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

// Starting rates: seconds per second of audio; translate is seconds per line, title is seconds per job.
// Apple Silicon numbers were measured on an M1 Pro (Metal). The CPU-only numbers (Windows, Intel Macs)
// are rough guesses for an office laptop. Each finished job blends its own numbers into rates.json.
const APPLE_SILICON: [(&str, f64); 7] = [
    ("prepare", 0.006), ("vad", 0.004), ("diarize", 0.21), ("transcribe", 0.15), ("name", 0.0005), ("title", 8.0), ("translate", 2.2),
];
const CPU_ONLY: [(&str, f64); 7] = [
    ("prepare", 0.01), ("vad", 0.006), ("diarize", 0.4), ("transcribe", 1.1), ("name", 0.001), ("title", 25.0), ("translate", 7.0),
];
/// Steps shown under "Transcription"; translation is its own phase.
pub const TRANSCRIPTION: [&str; 6] = ["prepare", "vad", "diarize", "transcribe", "name", "title"];
/// Steps whose rate is per second of audio.
const PER_AUDIO_SECOND: [&str; 5] = ["prepare", "vad", "diarize", "transcribe", "name"];

fn file(transcripts: &Path) -> PathBuf {
    transcripts.parent().unwrap_or(transcripts).join("rates.json")
}

fn load(transcripts: &Path) -> Map<String, Value> {
    let defaults = if cfg!(all(target_os = "macos", target_arch = "aarch64")) { APPLE_SILICON } else { CPU_ONLY };
    let mut m: Map<String, Value> = defaults.iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    let saved = fs::read_to_string(file(transcripts)).ok().and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok());
    m.extend(saved.unwrap_or_default());
    m
}

fn rate(r: &Map<String, Value>, k: &str) -> f64 {
    r.get(k).and_then(Value::as_f64).unwrap_or(0.0)
}

/// Expected seconds per step for `duration` seconds of audio and `lines` lines to translate.
pub fn per_step(transcripts: &Path, duration: f64, lines: f64, title: bool) -> Map<String, Value> {
    let r = load(transcripts);
    let mut m: Map<String, Value> = PER_AUDIO_SECOND.iter().map(|k| (k.to_string(), json!(rate(&r, k) * duration))).collect();
    m.insert("title".into(), json!(if title { rate(&r, "title") } else { 0.0 }));
    m.insert("translate".into(), json!(rate(&r, "translate") * lines));
    m
}

/// Lines we expect to translate before transcription has told us (about half of one line per 15s).
pub fn guess_lines(duration: f64, translate: bool) -> f64 {
    if translate { duration / 15.0 * 0.5 } else { 0.0 }
}

/// Expected seconds for transcription and for translation, shown before a job starts.
pub fn estimate(transcripts: &Path, duration: f64, translate: bool) -> Value {
    let est = per_step(transcripts, duration, guess_lines(duration, translate), true);
    let transcribe: f64 = TRANSCRIPTION.iter().map(|k| rate(&est, k)).sum();
    json!({"transcribe": transcribe, "translate": rate(&est, "translate")})
}

/// Blend this job's measured speed into the saved rates (half old, half new).
pub fn learn(transcripts: &Path, took: &Map<String, Value>, duration: f64, lines: usize) {
    let mut r = load(transcripts);
    for (k, secs) in took {
        let per = match k.as_str() {
            "translate" if lines > 0 => lines as f64,
            "title" => 1.0,
            k if PER_AUDIO_SECOND.contains(&k) && duration > 0.0 => duration,
            _ => continue,
        };
        let measured = secs.as_f64().unwrap_or(0.0) / per;
        r.insert(k.clone(), json!(0.5 * rate(&r, k) + 0.5 * measured));
    }
    let _ = fs::write(file(transcripts), serde_json::to_string_pretty(&r).unwrap_or_default());
}

pub fn now_iso() -> String {
    iso(std::time::SystemTime::now())
}

/// A moment as an ISO 8601 UTC string (no date library needed for a few timestamps).
pub fn iso(t: std::time::SystemTime) -> String {
    let secs = t.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil-from-days (Howard Hinnant)
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_looks_right() {
        let s = super::now_iso();
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("20") && s.ends_with('Z'), "{s}");
    }
}
