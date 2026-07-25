//! Diagnostic: what does Whisper actually return for the 3-second mic chunks
//! the live path feeds it, versus the whole file the post-stop path uses?
//!
//! Mic text appears after a recording stops but never live, and the gate is no
//! longer the blocker (7/52 chunks clear it), so this checks the next stage.
//!
//!   cargo run --example mic_chunk_probe -- <mic.wav> <model.bin>

use std::env;
use std::path::Path;

use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

const TARGET_PEAK: f32 = 0.3;
const MAX_GAIN: f32 = 8.0;
const GATE: f32 = 0.02;

fn rms(s: &[f32]) -> f32 {
    if s.is_empty() {
        return 0.0;
    }
    (s.iter().map(|x| x * x).sum::<f32>() / s.len() as f32).sqrt()
}

fn normalize_peak(samples: &mut [f32], target_peak: f32, max_gain: f32) {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak <= f32::EPSILON {
        return;
    }
    let gain = (target_peak / peak).clamp(1.0, max_gain);
    if gain > 1.0 {
        for s in samples.iter_mut() {
            *s *= gain;
        }
    }
}

/// Linear resample, matching the app's approach closely enough for this probe.
fn resample(input: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to {
        return input.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = (input.len() as f64 / ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let idx = src as usize;
            let frac = (src - idx as f64) as f32;
            let a = input.get(idx).copied().unwrap_or(0.0);
            let b = input.get(idx + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

fn transcribe(ctx: &WhisperContext, samples_16k: &[f32]) -> Vec<String> {
    let mut state = ctx.create_state().expect("state");
    let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
    params.set_language(None);
    params.set_translate(false);
    params.set_print_special(false);
    params.set_print_progress(false);
    params.set_print_realtime(false);
    params.set_print_timestamps(false);
    params.set_n_threads(8);

    if state.full(params, samples_16k).is_err() {
        return vec!["<whisper error>".into()];
    }
    let n = state.full_n_segments().unwrap_or(0);
    (0..n)
        .filter_map(|i| state.full_get_segment_text(i).ok())
        .collect()
}

fn main() {
    let args: Vec<String> = env::args().collect();
    let wav = args.get(1).expect("usage: mic_chunk_probe <wav> <model>");
    let model = args.get(2).expect("usage: mic_chunk_probe <wav> <model>");

    let mut reader = hound::WavReader::open(Path::new(wav)).expect("open wav");
    let spec = reader.spec();
    let raw: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| s.unwrap_or(0) as f32 / 32768.0)
        .collect();

    // Downmix to mono like the recorder does.
    let ch = spec.channels as usize;
    let mono: Vec<f32> = if ch > 1 {
        raw.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect()
    } else {
        raw
    };
    println!(
        "loaded {} ({} Hz, {} ch) -> {:.1}s mono",
        wav,
        spec.sample_rate,
        ch,
        mono.len() as f32 / spec.sample_rate as f32
    );

    let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
        .expect("load model");

    // --- The live path: 3-second chunks that clear the gate ---
    let step = (spec.sample_rate as usize) * 3;
    let mut tested = 0;
    println!("\n=== LIVE PATH: 3s chunks (normalized, gate {GATE}) ===");
    for (i, c) in mono.chunks(step).enumerate() {
        if c.len() < step {
            break;
        }
        let mut chunk = c.to_vec();
        normalize_peak(&mut chunk, TARGET_PEAK, MAX_GAIN);
        if rms(&chunk) <= GATE {
            continue;
        }
        let s16 = resample(&chunk, spec.sample_rate, 16000);
        let t0 = std::time::Instant::now();
        let out = transcribe(&ctx, &s16);
        let ms = t0.elapsed().as_millis();
        println!(
            "  chunk {:>3} (t={:>5.1}s, rms={:.4}) inference={:>5}ms -> {:?}",
            i,
            (i * step) as f32 / spec.sample_rate as f32,
            rms(&chunk),
            ms,
            out
        );
        tested += 1;
        if tested >= 6 {
            break;
        }
    }
    if tested == 0 {
        println!("  (no chunk cleared the gate)");
    }

    // --- Does inference cost scale with audio length, or is it fixed per call?
    // Decides whether shorter chunks actually buy lower latency.
    println!("\n=== COST vs CHUNK LENGTH (from t=75s, normalized) ===");
    let base = (spec.sample_rate as usize) * 75;
    for secs in [3usize, 6, 12, 24] {
        let n = (spec.sample_rate as usize) * secs;
        if base + n > mono.len() {
            continue;
        }
        let mut chunk = mono[base..base + n].to_vec();
        normalize_peak(&mut chunk, TARGET_PEAK, MAX_GAIN);
        let s16 = resample(&chunk, spec.sample_rate, 16000);
        let t0 = std::time::Instant::now();
        let out = transcribe(&ctx, &s16);
        println!(
            "  {:>2}s audio -> inference {:>5}ms  ({} segment(s))",
            secs,
            t0.elapsed().as_millis(),
            out.len()
        );
    }

    // --- The post-stop path: one long span, unnormalized ---
    println!("\n=== POST-STOP PATH: first 60s as one span (no normalization) ===");
    let span: Vec<f32> = mono.iter().copied().take(spec.sample_rate as usize * 60).collect();
    let s16 = resample(&span, spec.sample_rate, 16000);
    for line in transcribe(&ctx, &s16) {
        println!("  {line}");
    }
}
