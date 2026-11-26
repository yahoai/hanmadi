//! Recording -> transcript. Same engines as prototype/pipeline.py, driven as sidecar binaries:
//! ffmpeg (decode), sherpa-onnx (VAD, diarization), whisper.cpp (per-segment language + text),
//! llama.cpp (title, translation).
mod audio;
mod llm;
mod proc;
mod redo;
mod resume;
mod segments;
mod timing;

pub(crate) use proc::{exec, ffmpeg, wait_with};
use segments::Diar;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::time::Instant;
use tauri::{AppHandle, Emitter};

pub use proc::{preview, probe};
pub use redo::{retitle, retranslate, terms};
pub use resume::{any_live, resume, status};
pub use timing::estimate;

#[derive(Serialize, Deserialize, Clone)]
pub struct Speaker {
    pub name: String,
    pub langs: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Source {
    pub path: String,
    pub kind: String,
    /// When the recording was made (not when it was added to the app).
    pub recorded: String,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct JobReq {
    pub source: Source,
    pub title: String,
    pub speakers: Vec<Speaker>,
    pub vocab: Vec<String>,
    pub translate_to: Option<String>,
    /// Replace `title` with one the local model writes (the user didn't type one).
    #[serde(default)]
    pub auto_title: bool,
    /// Language for an automatic title; the app's UI language.
    #[serde(default)]
    pub title_lang: Option<String>,
}

static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());
/// Ids of jobs queued or running in this process. The status on disk can't tell: it stays
/// "running" forever if the app quit mid-job.
static LIVE: Mutex<BTreeSet<String>> = Mutex::new(BTreeSet::new());

pub fn is_live(id: &str) -> bool {
    LIVE.lock().unwrap_or_else(|e| e.into_inner()).contains(id)
}

// ponytail: engines and models are read from the dev checkout; bundle them as app resources for distribution
fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}
fn num(v: &Value, k: &str) -> f64 {
    v[k].as_f64().unwrap_or(0.0)
}
fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}

// (step, where its share of the progress bar starts, share)
const STEPS: [(&str, f64, f64); 7] = [
    ("prepare", 0.0, 0.03),
    ("vad", 0.03, 0.02),
    ("diarize", 0.05, 0.40),
    ("transcribe", 0.45, 0.40),
    ("name", 0.85, 0.01),
    ("title", 0.86, 0.02),
    ("translate", 0.88, 0.12),
];

struct Job {
    app: Option<AppHandle>, // None when run from the command line
    dir: PathBuf,
    doc: Value,
    started: Instant,
    step_started: Instant,
    took: Map<String, Value>,
    est: Map<String, Value>,
}

impl Job {
    fn save(&self) {
        let _ = fs::write(self.dir.join("transcript.json"), serde_json::to_string(&self.doc).unwrap_or_default());
        let mut ev = self.doc.clone();
        if let Some(o) = ev.as_object_mut() {
            o.remove("turns");
        }
        match &self.app {
            Some(app) => {
                let _ = app.emit("job-progress", ev);
            }
            None => {
                let p = &ev["progress"];
                eprintln!("{} {:.0}% {}", text(p, "step"), num(p, "pct") * 100.0, text(p, "detail"));
            }
        }
    }
    fn step(&mut self, name: &str) {
        if let Some(prev) = self.doc["progress"]["step"].as_str() {
            self.took.insert(prev.into(), json!(self.step_started.elapsed().as_secs_f64()));
        }
        self.step_started = Instant::now();
        self.at(name, 0.0, None);
    }
    fn at(&mut self, name: &str, frac: f64, detail: Option<String>) {
        let (_, base, share) = STEPS.iter().find(|s| s.0 == name).copied().unwrap_or(STEPS[0]);
        let pct = base + share * frac.clamp(0.0, 1.0);
        let step_el = self.step_started.elapsed().as_secs_f64();
        self.doc["progress"] = json!({
            "step": name, "pct": pct, "frac": frac, "stepEl": step_el, "detail": detail, "took": self.took, "est": self.est,
        });
        self.save();
    }
    fn transcripts(&self) -> PathBuf {
        self.dir.parent().unwrap_or(&self.dir).to_path_buf()
    }
}

pub fn start(app: AppHandle, transcripts: PathBuf, req: JobReq) -> Result<String, String> {
    launch(Some(app), &transcripts, req, None)
}

/// Same job, run in the foreground with progress on stderr (dev and batch use).
pub fn run_blocking(transcripts: PathBuf, req: JobReq) -> Result<String, String> {
    launch(None, &transcripts, req, None)
}

/// Queue a job on a background thread (app), or run it in the foreground (command line).
/// An existing `id` reruns that transcript in its folder, reusing what its work/ holds.
fn launch(app: Option<AppHandle>, transcripts: &Path, req: JobReq, id: Option<String>) -> Result<String, String> {
    let id = id.unwrap_or_else(|| std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis().to_string());
    let live = || LIVE.lock().unwrap_or_else(|e| e.into_inner());
    if !live().insert(id.clone()) {
        return Err("busy".into());
    }
    let mut job = new_job(app, transcripts, &req, &id).inspect_err(|_| {
        live().remove(&id);
    })?;
    if job.app.is_none() {
        work(&mut job, &req);
        live().remove(&id);
        return match job.doc["status"].as_str() {
            Some("done") => Ok(id),
            _ => Err(text(&job.doc, "error").to_string()),
        };
    }
    let done = id.clone();
    std::thread::spawn(move || {
        work(&mut job, &req);
        live().remove(&done);
    });
    Ok(id)
}

fn new_job(app: Option<AppHandle>, transcripts: &Path, req: &JobReq, id: &str) -> Result<Job, String> {
    let dir = transcripts.join(id);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // a rerun keeps when it was first added
    let before = fs::read_to_string(dir.join("transcript.json")).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok());
    let added = before.as_ref().and_then(|d| d["added"].as_str()).map(String::from).unwrap_or_else(timing::now_iso);
    let doc = json!({
        "id": id, "title": req.title, "recorded": req.source.recorded, "added": added, "duration": 0.0,
        "source": {"kind": req.source.kind, "path": req.source.path},
        "status": "queued", "speakers": [], "vocab": req.vocab,
        "translateTo": req.translate_to.clone().filter(|s| !s.is_empty()),
        "languages": [], "turns": [], "diar": [], "audio": "audio.m4a",
        "progress": {"step": "prepare", "pct": 0.0},
        "req": req,
    });
    let (started, step_started, took, est) = (Instant::now(), Instant::now(), Map::new(), Map::new());
    let job = Job { app, dir, doc, started, step_started, took, est };
    job.save();
    Ok(job)
}

fn work(job: &mut Job, req: &JobReq) {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(|e| e.into_inner());
    job.started = Instant::now();
    job.doc["status"] = json!("running");
    if let Err(e) = run(job, req) {
        job.doc["status"] = json!("error");
        job.doc["error"] = json!(e);
        job.save();
    }
}

fn run(job: &mut Job, req: &JobReq) -> Result<(), String> {
    let work = job.dir.join("work");
    // a stopped run left work/ behind: reuse it, but its near-zero step times would skew the learned rates
    let resumed = work.exists();
    fs::create_dir_all(&work).map_err(|e| e.to_string())?;
    let (wav, m4a, vad_json) = (work.join("audio16k.wav"), job.dir.join("audio.m4a"), work.join("vad.json"));

    job.step("prepare");
    let dec = |extra: &[&str], out: &Path| {
        let base = ["-loglevel", "error", "-y", "-i", &req.source.path, "-map", "0:a:0", "-ac", "1"];
        exec(Command::new(ffmpeg()).args(base).args(extra).arg(out))
    };
    let full = |p: &Path| fs::metadata(p).is_ok_and(|m| m.len() > 0);
    // vad.json is written after both decodes, so files from a killed ffmpeg are never trusted
    if !(full(&wav) && full(&m4a) && vad_json.exists()) {
        dec(&["-ar", "16000"], &wav)?;
        dec(&["-c:a", "aac", "-b:a", "64k"], &m4a)?;
    }
    let pcm = audio::read_wav(&wav)?;
    let duration = pcm.len() as f64 / audio::RATE;
    job.doc["duration"] = json!(duration);
    let lines_guess = timing::guess_lines(duration, job.doc["translateTo"].is_string());
    job.est = timing::per_step(&job.transcripts(), duration, lines_guess, req.auto_title);

    job.step("vad");
    let vad = resume::cached(&vad_json, || detect_speech(&wav, &work))?;
    job.step("diarize");
    let diar = resume::cached(&work.join("diar.json"), || diarize(job, &wav, req.speakers.len()))?;
    let pieces = segments::merge_same_speaker(segments::split_at_turns(&vad, &diar), &diar);
    job.step("transcribe");
    let mut turns = transcribe(job, req, &pieces, &pcm, &work)?;

    job.step("name");
    let n_clusters = diar.iter().map(|d| d.2 + 1).max().unwrap_or(1);
    let names = segments::name_speakers(&turns, &req.speakers, n_clusters);
    job.doc["speakers"] = json!(names.iter().map(|s| json!({"name": s.name, "langs": s.langs})).collect::<Vec<_>>());
    language_steps(job, req, &mut turns, &names)?;
    audio::waveforms(&mut turns, &pcm);
    finish(job, turns, &diar, !resumed);
    let _ = fs::remove_dir_all(&work);
    Ok(())
}

/// Title, then translation. Both use the same local model, started once.
fn language_steps(job: &mut Job, req: &JobReq, turns: &mut [Value], names: &[Speaker]) -> Result<(), String> {
    let to = job.doc["translateTo"].as_str().map(String::from);
    let lines = to.as_ref().map_or(0, |to| turns.iter().filter(|t| text(t, "l") != to.as_str()).count());
    job.est["translate"] = timing::per_step(&job.transcripts(), 0.0, lines as f64, false)["translate"].clone();
    if req.auto_title {
        job.step("title");
    }
    // a failed title or term list is not worth failing the transcript: keep the recording's name, suggest nothing
    let mut server = llm::start().map_err(|e| eprintln!("{e}")).ok();
    if let Some(s) = &server {
        let lang = req.title_lang.clone().unwrap_or_else(|| "en".into());
        if let Some(Ok(t)) = req.auto_title.then(|| llm::title(s, turns, &lang, &req.vocab, names)) {
            job.doc["title"] = json!(t);
        }
        match llm::terms(s, turns, &req.vocab) {
            Ok(t) => job.doc["terms"] = json!(t),
            Err(e) => eprintln!("terms: {e}"),
        }
    }
    if let (Some(to), true) = (to, lines > 0) {
        job.step("translate");
        let server = match server.take() {
            Some(s) => s,
            None => llm::start()?,
        };
        llm::translate(&server, turns, &to, &req.vocab, names, |k, n| {
            job.at("translate", k as f64 / n as f64, Some(format!("{k} / {n}")))
        })?;
    }
    Ok(())
}

fn detect_speech(wav: &Path, work: &Path) -> Result<Vec<(f64, f64)>, String> {
    let r = root();
    let out = exec(
        Command::new(r.join("tools/sherpa/bin/sherpa-onnx-vad"))
            .arg(format!("--silero-vad-model={}", r.join("models/silero_vad.onnx").display()))
            .arg(wav)
            .arg(work.join("vad.wav")),
    )?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let (a, b) = l.split_once(" -- ")?;
            Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
        })
        .filter(|(a, b): &(f64, f64)| b - a >= segments::MIN_SEG)
        .collect())
}

fn diarize(job: &mut Job, wav: &Path, n_named: usize) -> Result<Vec<Diar>, String> {
    let r = root();
    let model = |p: &str| r.join("models").join(p).display().to_string();
    let mut cmd = Command::new(r.join("tools/sherpa/bin/sherpa-onnx-offline-speaker-diarization"));
    cmd.arg(if n_named >= 2 { format!("--clustering.num-clusters={n_named}") } else { "--clustering.cluster-threshold=0.9".into() })
        .arg(format!("--segmentation.pyannote-model={}", model("sherpa-onnx-pyannote-segmentation-3-0/model.onnx")))
        .arg(format!("--embedding.model={}", model("3dspeaker_speech_eres2net_base_sv_zh-cn_3dspeaker_16k.onnx")))
        .args(["--segmentation.num-threads=4", "--embedding.num-threads=4"])
        .arg(wav)
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let expected = job.est.get("diarize").and_then(Value::as_f64).unwrap_or(1.0).max(1.0);
    let child = cmd.spawn().map_err(|e| e.to_string())?;
    let out = wait_with(job, child, |job, el| job.at("diarize", el / expected, None))?;
    Ok(out
        .lines()
        .filter_map(|l| {
            let p: Vec<&str> = l.split_whitespace().collect();
            if p.len() != 4 || p[1] != "--" {
                return None;
            }
            Some((p[0].parse().ok()?, p[2].parse().ok()?, p[3].strip_prefix("speaker_")?.parse().ok()?))
        })
        .collect())
}

fn transcribe(job: &mut Job, req: &JobReq, pieces: &[Diar], pcm: &[i16], work: &Path) -> Result<Vec<Value>, String> {
    let segs = work.join("segs");
    // results from a stopped run are only kept when they were cut from the same pieces
    let cut = serde_json::to_string(pieces).unwrap_or_default();
    if fs::read_to_string(segs.join("pieces.json")).ok().as_ref() != Some(&cut) {
        let _ = fs::remove_dir_all(&segs);
    }
    fs::create_dir_all(&segs).map_err(|e| e.to_string())?;
    let _ = fs::write(segs.join("pieces.json"), &cut);
    let json_of = |p: &Path| PathBuf::from(format!("{}.json", p.display()));
    let heard = |p: &Path| fs::read_to_string(json_of(p)).ok().and_then(|s| serde_json::from_str::<Value>(&s).ok()).is_some();
    let paths: Vec<PathBuf> = (0..pieces.len()).map(|i| segs.join(format!("seg{i:05}.wav"))).collect();
    let todo: Vec<&PathBuf> = pieces
        .iter()
        .zip(&paths)
        .filter(|(_, path)| !heard(path))
        .map(|(p, path)| {
            let _ = audio::write_wav(path, audio::slice(pcm, p.0, p.1));
            path
        })
        .collect();
    let skipped = paths.len() - todo.len();
    let threads = std::thread::available_parallelism().map(|n| n.get().min(8)).unwrap_or(4).to_string();
    // ponytail: 100 files per call keeps the command line under Windows' 32K limit; costs one model load per batch
    for (bi, batch) in todo.chunks(100).enumerate() {
        let mut cmd = Command::new(root().join("tools/whisper.cpp/build/bin/whisper-cli"));
        cmd.arg("-m").arg(root().join("models/ggml-large-v3-turbo-q5_0.bin"));
        cmd.args(["-l", "auto", "-oj", "-np", "-t", &threads, "-mc", "0", "-sns"]);
        if !req.vocab.is_empty() {
            cmd.args(["--prompt", &req.vocab.join(", ")]);
        }
        cmd.args(batch).stdout(Stdio::null()).stderr(Stdio::null());
        let child = cmd.spawn().map_err(|e| e.to_string())?;
        wait_with(job, child, |job, _| {
            let done = skipped + bi * 100 + batch.iter().filter(|p| json_of(p).exists()).count();
            job.at("transcribe", done as f64 / paths.len() as f64, Some(format!("{done} / {}", paths.len())));
        })?;
    }
    let allowed: Vec<&str> = req.speakers.iter().flat_map(|s| s.langs.iter().map(String::as_str)).chain(["en"]).collect();
    let mut turns: Vec<Value> = vec![];
    for (p, path) in pieces.iter().zip(&paths) {
        let j: Value = fs::read_to_string(json_of(path)).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let lang = text(&j["result"], "language").to_string();
        let words: Vec<&str> = j["transcription"].as_array().into_iter().flatten().map(|x| text(x, "text").trim()).collect();
        let line = segments::dedupe_loops(words.join(" ").trim());
        // short lines in a language nobody here speaks are whisper hallucinations ("Thank you.", "Subscribe!")
        if line.is_empty() || (allowed.len() > 1 && !allowed.contains(&lang.as_str()) && p.1 - p.0 < 3.0) {
            continue;
        }
        match turns.last_mut() {
            Some(last) if last["s"] == p.2 && text(last, "l") == lang && p.0 - num(last, "e") < 2.0 => {
                last["x"] = json!(format!("{} {}", text(last, "x"), line));
                last["e"] = json!(p.1);
            }
            _ => turns.push(json!({"t": p.0, "e": p.1, "s": p.2, "l": lang, "x": line})),
        }
    }
    Ok(turns)
}

fn finish(job: &mut Job, turns: Vec<Value>, diar: &[Diar], learn: bool) {
    let mut by_lang: HashMap<String, f64> = HashMap::new();
    for t in &turns {
        *by_lang.entry(text(t, "l").to_string()).or_default() += num(t, "e") - num(t, "t");
    }
    let mut langs: Vec<_> = by_lang.into_iter().collect();
    langs.sort_by(|a, b| b.1.total_cmp(&a.1));
    job.doc["languages"] = json!(langs.into_iter().map(|l| l.0).collect::<Vec<_>>());
    job.doc["diar"] = json!(diar.iter().map(|d| json!([d.0, d.1, d.2])).collect::<Vec<_>>());
    if let Some(last) = job.doc["progress"]["step"].as_str() {
        job.took.insert(last.into(), json!(job.step_started.elapsed().as_secs_f64()));
    }
    let lines = turns.iter().filter(|t| t.get("tr").is_some()).count();
    if learn {
        timing::learn(&job.transcripts(), &job.took, num(&job.doc, "duration"), lines);
    }
    let mut took = job.took.clone();
    took.insert("total".into(), json!(job.started.elapsed().as_secs_f64()));
    job.doc["timing"] = json!(took);
    job.doc["turns"] = json!(turns);
    job.doc["status"] = json!("done");
    if let Some(o) = job.doc.as_object_mut() {
        o.remove("progress");
    }
    job.save();
    if let Some(app) = &job.app {
        use tauri_plugin_notification::NotificationExt;
        let _ = app.notification().builder().title("Transcript ready").body(text(&job.doc, "title")).show();
    }
}
