//! 16 kHz mono PCM in and out of WAV, and the per-line waveform bars shown next to each transcript line.
use super::num;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;

pub const RATE: f64 = 16000.0;

pub fn read_wav(p: &Path) -> Result<Vec<i16>, String> {
    let b = fs::read(p).map_err(|e| e.to_string())?;
    let mut i = 12;
    while i + 8 <= b.len() {
        let n = u32::from_le_bytes([b[i + 4], b[i + 5], b[i + 6], b[i + 7]]) as usize;
        if &b[i..i + 4] == b"data" {
            let end = (i + 8 + n).min(b.len());
            return Ok(b[i + 8..end]
                .chunks_exact(2)
                .map(|c| i16::from_le_bytes([c[0], c[1]]))
                .collect());
        }
        i += 8 + n + (n & 1);
    }
    Err("no audio data in decoded wav".into())
}

pub fn write_wav(p: &Path, s: &[i16]) -> std::io::Result<()> {
    let n = (s.len() * 2) as u32;
    let mut b = Vec::with_capacity(44 + n as usize);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    for v in [16u32.to_le_bytes(), [1, 0, 1, 0], 16000u32.to_le_bytes(), 32000u32.to_le_bytes(), [2, 0, 16, 0]] {
        b.extend_from_slice(&v);
    }
    b.extend_from_slice(b"data");
    b.extend_from_slice(&n.to_le_bytes());
    for x in s {
        b.extend_from_slice(&x.to_le_bytes());
    }
    fs::write(p, b)
}

/// Samples between two times, clamped to the recording.
pub fn slice(pcm: &[i16], a: f64, b: f64) -> &[i16] {
    let e = ((b * RATE) as usize).min(pcm.len());
    &pcm[((a * RATE) as usize).min(e)..e]
}

/// 10 to 56 bars per line (longer lines look longer), heights 0..9 on a shared scale.
pub fn waveforms(turns: &mut [Value], pcm: &[i16]) {
    let per: Vec<Vec<f64>> = turns
        .iter()
        .map(|t| {
            let w = slice(pcm, num(t, "t"), num(t, "e"));
            let bars = ((num(t, "e") - num(t, "t")) * 2.2).round().clamp(10.0, 56.0) as usize;
            let step = (w.len() / bars).max(1);
            (0..bars)
                .map(|i| {
                    let chunk = &w[(i * step).min(w.len())..((i + 1) * step).min(w.len())];
                    let n = chunk.iter().step_by(8).count().max(1) as f64;
                    (chunk.iter().step_by(8).map(|&x| (x as f64).powi(2)).sum::<f64>() / n).sqrt()
                })
                .collect()
        })
        .collect();
    let mut all: Vec<f64> = per.iter().flatten().copied().collect();
    all.sort_by(|a, b| a.total_cmp(b));
    let top = all.get(all.len() * 97 / 100).copied().unwrap_or(1.0).max(1.0);
    for (t, v) in turns.iter_mut().zip(per) {
        t["w"] = json!(v.iter().map(|x| (9.0 * (x / top).sqrt()).round().min(9.0) as u8).collect::<Vec<_>>());
    }
}
