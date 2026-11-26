//! Local LLM (Qwen3 4B Instruct on llama.cpp) for translation and titles. Nothing leaves the machine.
use super::{root, text, Speaker};
use serde_json::{json, Value};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

pub struct Server {
    child: Child,
    base: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

pub fn start() -> Result<Server, String> {
    let r = root();
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map_err(|e| e.to_string())?.port();
    let gpu_layers = if cfg!(all(target_os = "macos", target_arch = "aarch64")) { "99" } else { "0" };
    let child = Command::new(r.join("tools/llama.cpp/build/bin/llama-server"))
        .arg("-m")
        .arg(r.join("models/Qwen3-4B-Instruct-2507-Q4_K_M.gguf"))
        .args(["--port", &port.to_string(), "-c", "4096", "-ngl", gpu_layers, "--log-disable"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("language model: {e}"))?;
    let server = Server { child, base: format!("http://127.0.0.1:{port}") };
    let up = Instant::now();
    while ureq::get(&format!("{}/health", server.base)).call().is_err() {
        if up.elapsed() > Duration::from_secs(120) {
            return Err("language model did not start".into());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Ok(server)
}

impl Server {
    fn chat(&self, system: &str, user: &str, max_tokens: u32) -> Result<String, String> {
        let body = json!({
            "messages": [{"role": "system", "content": system}, {"role": "user", "content": user}],
            "temperature": 0.2, "max_tokens": max_tokens,
        });
        let res: Value = ureq::post(&format!("{}/v1/chat/completions", self.base))
            .send_json(body)
            .map_err(|e| e.to_string())?
            .into_json()
            .map_err(|e| e.to_string())?;
        Ok(text(&res["choices"][0]["message"], "content").trim().to_string())
    }
}

pub fn language_name(code: &str) -> &str {
    match code {
        "ko" => "Korean",
        "en" => "English",
        "zh" => "Simplified Chinese",
        "ja" => "Japanese",
        other => other,
    }
}

fn who(names: &[Speaker], t: &Value) -> String {
    let i = t["s"].as_u64().unwrap_or(0) as usize;
    names.get(i).map(|s| s.name.clone()).filter(|n| !n.is_empty()).unwrap_or_else(|| "Speaker".into())
}

/// Adds `tr` to every turn not already in `to`. `progress(done, total)` is called after each line.
pub fn translate(llm: &Server, turns: &mut [Value], to: &str, vocab: &[String], names: &[Speaker], mut progress: impl FnMut(usize, usize)) -> Result<(), String> {
    let todo: Vec<usize> = (0..turns.len()).filter(|&i| text(&turns[i], "l") != to).collect();
    for (k, &i) in todo.iter().enumerate() {
        let context: Vec<String> = turns[i.saturating_sub(3)..i].iter().map(|t| format!("{}: {}", who(names, t), text(t, "x"))).collect();
        let system = format!(
            "You translate lines from a multilingual meeting transcript into natural {}. \
             Reply with the translation of the user's line only, no notes or labels. \
             Keep personal names, company and product names as they are. \
             The text is machine transcribed and may contain recognition errors: translate the intended meaning.\n\
             Terms used in this meeting: {}\n\nEarlier lines, for context only:\n{}",
            language_name(to),
            vocab.join(", "),
            context.join("\n")
        );
        let tr = llm.chat(&system, text(&turns[i], "x"), 400)?;
        turns[i]["tr"] = json!(tr);
        progress(k + 1, todo.len());
    }
    Ok(())
}

/// Lines spread evenly across the meeting, about 2400 characters: keeps the prompt well inside the 4K context.
fn sample(lines: &[String]) -> String {
    let total: usize = lines.iter().map(|l| l.chars().count()).sum::<usize>().max(1);
    lines.iter().step_by(total.div_ceil(2400).max(1)).map(String::as_str).collect::<Vec<_>>().join("\n")
}

/// Vocabulary words actually said in these lines. The whole list made the model title a family chat after a client meeting.
fn said(vocab: &[String], lines: &[String]) -> Vec<String> {
    let all = lines.join("\n").to_lowercase();
    vocab.iter().filter(|w| all.contains(&w.to_lowercase())).cloned().collect()
}

/// A short meeting title in `lang`, from lines sampled across the whole meeting.
pub fn title(llm: &Server, turns: &[Value], lang: &str, vocab: &[String], names: &[Speaker]) -> Result<String, String> {
    let lines: Vec<String> = turns.iter().map(|t| format!("{}: {}", who(names, t), t.get("tr").and_then(Value::as_str).unwrap_or(text(t, "x")))).collect();
    let system = format!(
        "You name recordings. Write one short title in {} (at most 8 words) that says what is talked about. \
         Call it a meeting only if it is a work meeting; a casual or family conversation is not one. \
         Use names that are actually said in it. Reply with the title only: no quotes, no date, no trailing period.\n\
         Spell these names exactly like this: {}",
        language_name(lang),
        said(vocab, &lines).join(", ")
    );
    let t = llm.chat(&system, &sample(&lines), 40)?;
    let t = t.lines().next().unwrap_or("").trim_matches(|c: char| c == '"' || c == '\'' || c == '「' || c == '」' || c.is_whitespace());
    if t.is_empty() { Err("empty title".into()) } else { Ok(t.trim_end_matches('.').to_string()) }
}

/// Names, products, acronyms and jargon said in the meeting, in their correct spelling, for the vocabulary.
// ponytail: one call on the 2400-character sample, so rare terms in long meetings are missed; chunk the transcript if that matters
pub fn terms(llm: &Server, turns: &[Value], vocab: &[String]) -> Result<Vec<String>, String> {
    let lines: Vec<String> = turns.iter().map(|t| text(t, "x").to_string()).collect();
    let system = format!(
        "You read a machine transcribed meeting and list the proper nouns and specialist terms used in it: \
         names of people, companies, organizations, products, projects and places, acronyms and technical jargon. \
         Never list common nouns or everyday words (family words, food, feelings, objects). Only list terms that appear in the text. \
         If there are none, reply with nothing. \
         List at most 15, one per line, with no numbering, notes or translations.\n\
         Spell these exactly like this if they appear: {}",
        said(vocab, &lines).join(", ")
    );
    // only what is really in the transcript; a misheard spelling is fixed by the user, which also fixes the text
    let all = lines.join("\n").to_lowercase();
    Ok(parse_terms(&llm.chat(&system, &sample(&lines), 200)?).into_iter().filter(|t| all.contains(&t.to_lowercase())).collect())
}

/// One term per line or comma. Drops bullets, numbering, quotes, repeats (any case) and anything over 40 characters.
fn parse_terms(reply: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for item in reply.split(['\n', ',', '、', ';']) {
        let s = item.trim().trim_start_matches(['-', '*', '•']).trim();
        let s = match s.split_once(['.', ')']) {
            Some((n, rest)) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) && rest.starts_with(' ') => rest,
            _ => s,
        };
        let s = s.trim().trim_matches(['"', '\'', '`', '*']).trim();
        let n = s.chars().count();
        if (2..=40).contains(&n) && !out.iter().any(|o| o.to_lowercase() == s.to_lowercase()) {
            out.push(s.to_string());
        }
    }
    out.truncate(15);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn parse_terms_cleans_the_list() {
        let reply = "1. Nimbus\n2) Orion\n- orion\n* \"Priya\"\nPMS, RMP\n3M\nA product name that is far too long to be a term\n";
        assert_eq!(super::parse_terms(reply), ["Nimbus", "Orion", "Priya", "PMS", "RMP", "3M"]);
    }

    #[test]
    fn said_keeps_only_words_in_the_text() {
        let v = ["Nimbus", "Orion", "PMS"].map(String::from);
        let lines = ["we use the nimbus adapter".to_string(), "그걸 샤베트로".to_string()];
        assert_eq!(super::said(&v, &lines), ["Nimbus"]);
    }
}
