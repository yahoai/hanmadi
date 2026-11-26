//! Work on transcripts that are already done: translate them again, give them a title.
use super::{llm, timing, Speaker};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;
use std::time::Instant;

fn load(dir: &Path) -> Result<Value, String> {
    let raw = fs::read_to_string(dir.join("transcript.json")).map_err(|e| e.to_string())?;
    serde_json::from_str(&raw).map_err(|e| e.to_string())
}

fn save(dir: &Path, doc: &Value) -> Result<(), String> {
    fs::write(dir.join("transcript.json"), serde_json::to_string(doc).unwrap_or_default()).map_err(|e| e.to_string())
}

fn hints(doc: &Value) -> (Vec<Speaker>, Vec<String>) {
    let names = serde_json::from_value(doc["speakers"].clone()).unwrap_or_default();
    let vocab = serde_json::from_value(doc["vocab"].clone()).unwrap_or_default();
    (names, vocab)
}

/// Translate (or re-translate) a finished transcript into `to`.
pub fn retranslate(dir: &Path, to: &str) -> Result<(), String> {
    let mut doc = load(dir)?;
    let (names, vocab) = hints(&doc);
    let mut turns = doc["turns"].as_array().cloned().unwrap_or_default();
    for t in turns.iter_mut().filter_map(Value::as_object_mut) {
        t.remove("tr");
    }
    let started = Instant::now();
    let server = llm::start()?;
    llm::translate(&server, &mut turns, to, &vocab, &names, |k, n| {
        if k % 10 == 0 || k == n {
            eprintln!("translate {k} / {n}");
        }
    })?;
    let mut took = Map::new();
    took.insert("translate".into(), json!(started.elapsed().as_secs_f64()));
    let lines = turns.iter().filter(|t| t.get("tr").is_some()).count();
    timing::learn(dir.parent().unwrap_or(dir), &took, 0.0, lines);
    if !doc["timing"].is_object() {
        doc["timing"] = json!({});
    }
    doc["timing"]["translate"] = took["translate"].clone();
    doc["turns"] = json!(turns);
    doc["translateTo"] = json!(to);
    save(dir, &doc)
}

/// Ask the local model for a title in `lang` and store it.
pub fn retitle(dir: &Path, lang: &str) -> Result<String, String> {
    let mut doc = load(dir)?;
    let (names, vocab) = hints(&doc);
    let turns = doc["turns"].as_array().cloned().unwrap_or_default();
    let server = llm::start()?;
    let title = llm::title(&server, &turns, lang, &vocab, &names)?;
    doc["title"] = json!(title);
    save(dir, &doc)?;
    Ok(title)
}

/// Ask the local model for vocabulary found in a finished transcript and store it as `terms`.
pub fn terms(dir: &Path) -> Result<Vec<String>, String> {
    let doc = load(dir)?;
    let server = llm::start()?;
    let terms = llm::terms(&server, doc["turns"].as_array().map_or(&[][..], |t| t), &hints(&doc).1)?;
    // read again: the user may have edited and saved while the model ran
    let mut doc = load(dir)?;
    doc["terms"] = json!(terms);
    save(dir, &doc)?;
    Ok(terms)
}
