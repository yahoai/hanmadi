//! Turning VAD + diarization into the pieces whisper sees, cleaning whisper text, and naming speakers.
use super::{num, text, Speaker};
use serde_json::Value;
use std::collections::HashMap;

pub const MIN_SEG: f64 = 0.6; // shorter blips make whisper hallucinate
const MAX_MERGED: f64 = 25.0; // whisper window is 30s

pub type Diar = (f64, f64, usize);

/// VAD keeps quick back-and-forth in one segment: cut again where diarization says the speaker changes.
pub fn split_at_turns(vad: &[(f64, f64)], diar: &[Diar]) -> Vec<(f64, f64)> {
    let mut out = vec![];
    for &(a, b) in vad {
        let mut over: Vec<_> = diar.iter().filter(|d| d.1 > a && d.0 < b).collect();
        over.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut edges = vec![a];
        for w in over.windows(2) {
            if w[1].2 != w[0].2 && w[1].0 > a + MIN_SEG && w[1].0 < b - MIN_SEG {
                edges.push(w[1].0);
            }
        }
        edges.push(b);
        out.extend(edges.windows(2).map(|e| (e[0], e[1])).filter(|(p, q)| q - p >= MIN_SEG));
    }
    out
}

fn speaker_of(a: f64, b: f64, diar: &[Diar]) -> usize {
    let mut ov: HashMap<usize, f64> = HashMap::new();
    for d in diar {
        *ov.entry(d.2).or_default() += (b.min(d.1) - a.max(d.0)).max(0.0);
    }
    ov.into_iter().max_by(|x, y| x.1.total_cmp(&y.1)).map(|x| x.0).unwrap_or(0)
}

/// Re-join same-speaker pieces up to 25s so whisper gets enough audio to detect the language.
pub fn merge_same_speaker(pieces: Vec<(f64, f64)>, diar: &[Diar]) -> Vec<Diar> {
    let mut out: Vec<Diar> = vec![];
    for (a, b) in pieces {
        let s = speaker_of(a, b, diar);
        match out.last_mut() {
            Some(l) if l.2 == s && a - l.1 < 1.0 && b - l.0 <= MAX_MERGED => l.1 = b,
            _ => out.push((a, b, s)),
        }
    }
    out
}

/// whisper decoding loops ("对,对,对,..."): keep at most 3 repeats of any unit up to 12 chars.
pub fn dedupe_loops(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out: Vec<char> = vec![];
    let mut i = 0;
    'next: while i < chars.len() {
        for unit in 1..=12 {
            let n = out.len();
            if n < unit * 3 || i + unit > chars.len() {
                continue;
            }
            let tail = &out[n - unit..];
            if chars[i..i + unit] == *tail && out[n - 2 * unit..n - unit] == *tail && out[n - 3 * unit..n - 2 * unit] == *tail {
                i += unit;
                continue 'next;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out.into_iter().collect()
}

/// Seconds spoken per language for each cluster, and when each cluster first speaks.
fn profile(turns: &[Value], n: usize) -> (Vec<HashMap<String, f64>>, Vec<f64>) {
    let mut secs = vec![HashMap::new(); n];
    let mut first = vec![f64::MAX; n];
    for t in turns {
        let s = t["s"].as_u64().unwrap_or(0) as usize;
        if s < n {
            *secs[s].entry(text(t, "l").to_string()).or_default() += num(t, "e") - num(t, "t");
            first[s] = first[s].min(num(t, "t"));
        }
    }
    (secs, first)
}

/// Cluster -> person: the mapping that puts the fewest seconds in a language that person doesn't speak.
/// Names without languages are handed out in order of who speaks first.
pub fn name_speakers(turns: &[Value], hints: &[Speaker], n: usize) -> Vec<Speaker> {
    let blank = Speaker { name: String::new(), langs: vec![] };
    let (secs, first) = profile(turns, n);
    if hints.iter().all(|h| h.langs.is_empty()) {
        let mut order: Vec<usize> = (0..n).collect();
        order.sort_by(|a, b| first[*a].total_cmp(&first[*b]));
        let mut out = vec![blank; n];
        for (c, h) in order.into_iter().zip(hints) {
            out[c] = h.clone();
        }
        return out;
    }
    let cost: Vec<Vec<f64>> = (0..n)
        .map(|c| hints.iter().map(|h| secs[c].iter().filter(|(l, _)| !h.langs.contains(l)).map(|(_, v)| v).sum()).collect())
        .collect();
    let mut best = (f64::MAX, vec![None; n]);
    assign(0, &cost, &mut vec![false; hints.len()], &mut vec![None; n], 0.0, &mut best);
    best.1.into_iter().map(|h| h.map_or(blank.clone(), |h| hints[h].clone())).collect()
}

/// Exhaustive search; a cluster may stay unnamed only when there are more clusters than names.
fn assign(c: usize, cost: &[Vec<f64>], used: &mut Vec<bool>, cur: &mut Vec<Option<usize>>, acc: f64, best: &mut (f64, Vec<Option<usize>>)) {
    if acc >= best.0 {
        return;
    }
    if c == cost.len() {
        *best = (acc, cur.clone());
        return;
    }
    for h in 0..used.len() {
        if !used[h] {
            used[h] = true;
            cur[c] = Some(h);
            assign(c + 1, cost, used, cur, acc + cost[c][h], best);
            used[h] = false;
            cur[c] = None;
        }
    }
    let free_names = used.iter().filter(|u| !**u).count();
    if free_names < cost.len() - c {
        assign(c + 1, cost, used, cur, acc, best);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loops_and_turns() {
        assert_eq!(dedupe_loops("对,对,对,对,对,对,对,对"), "对,对,对,对");
        assert_eq!(dedupe_loops("네 네 알겠습니다"), "네 네 알겠습니다");
        let d = [(0.0, 5.0, 0), (5.0, 9.0, 1), (9.0, 9.3, 0)];
        assert_eq!(split_at_turns(&[(0.0, 9.2)], &d), vec![(0.0, 5.0), (5.0, 9.2)]);
        let pieces = vec![(0.0, 2.0), (2.5, 4.0), (5.5, 9.0)];
        assert_eq!(merge_same_speaker(pieces, &[(0.0, 4.0, 0), (5.0, 9.0, 1)]), vec![(0.0, 4.0, 0), (5.5, 9.0, 1)]);
    }

    #[test]
    fn names_by_language() {
        let t = |s: usize, l: &str, a: f64, b: f64| json!({"s": s, "l": l, "t": a, "e": b});
        let turns = [t(0, "zh", 0.0, 10.0), t(0, "ko", 10.0, 20.0), t(1, "zh", 20.0, 30.0), t(2, "ko", 30.0, 40.0)];
        let sp = |n: &str, l: &[&str]| Speaker { name: n.into(), langs: l.iter().map(|x| x.to_string()).collect() };
        let hints = [sp("Wei", &["zh"]), sp("Kim", &["ko"]), sp("Mina", &["ko", "zh"])];
        let names: Vec<String> = name_speakers(&turns, &hints, 3).into_iter().map(|s| s.name).collect();
        assert_eq!(names, ["Mina", "Wei", "Kim"]);
        let more: Vec<String> = name_speakers(&turns, &hints[..2], 3).into_iter().map(|s| s.name).collect();
        assert_eq!(more, ["", "Wei", "Kim"]);
    }
}
