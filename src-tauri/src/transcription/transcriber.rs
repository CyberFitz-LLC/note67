use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

use super::TranscriptionError;

/// A segment of transcribed text with timestamps
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionSegment {
    pub start_time: f64,
    pub end_time: f64,
    pub text: String,
    /// Who was speaking, when the recogniser could tell.
    ///
    /// Always `None` from local Whisper, which hears one voice per track and
    /// has no diarizer — attribution there comes from which file a segment was
    /// read out of, and is attached further up. A remote recogniser that
    /// diarizes fills this in with `Speaker 1..N`: placeholders, not names.
    #[serde(default)]
    pub speaker: Option<String>,
}

/// Result of a transcription
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionResult {
    pub segments: Vec<TranscriptionSegment>,
    pub full_text: String,
    pub language: Option<String>,
}

/// Transcriber for audio files using Whisper
pub struct Transcriber {
    /// Shared with live transcription. Whisper keeps model weights on the
    /// context and per-run scratch on the states created from it, so one
    /// context serves both callers; loading a second one would put another
    /// copy of the weights (~900MB for large-v3-turbo-q8) in memory for no
    /// benefit.
    ctx: Arc<WhisperContext>,
    is_transcribing: AtomicBool,
}

impl Transcriber {
    /// Load a Whisper context from a model file.
    ///
    /// Exposed so the caller can load once and hand the same context to both
    /// this transcriber and live transcription.
    pub fn load_context(model_path: &Path) -> Result<WhisperContext, TranscriptionError> {
        if !model_path.exists() {
            return Err(TranscriptionError::ModelNotFound(
                model_path.to_string_lossy().to_string(),
            ));
        }

        WhisperContext::new_with_params(
            model_path.to_str().unwrap(),
            // Defaults request GPU when a GPU backend was compiled in (see the
            // gpu-vulkan / gpu-cuda features) and fall back to CPU otherwise.
            WhisperContextParameters::default(),
        )
        .map_err(|e| TranscriptionError::ModelLoadError(e.to_string()))
    }

    /// Build a transcriber over an already-loaded context.
    pub fn from_context(ctx: Arc<WhisperContext>) -> Self {
        Self {
            ctx,
            is_transcribing: AtomicBool::new(false),
        }
    }

    /// Create a new transcriber with the specified model, loading its own
    /// context.
    pub fn new(model_path: &Path) -> Result<Self, TranscriptionError> {
        Ok(Self::from_context(Arc::new(Self::load_context(model_path)?)))
    }

    /// The shared context, so the caller can reuse it for live transcription.
    pub fn context(&self) -> Arc<WhisperContext> {
        Arc::clone(&self.ctx)
    }

    /// Check if currently transcribing
    #[allow(dead_code)]
    pub fn is_transcribing(&self) -> bool {
        self.is_transcribing.load(Ordering::SeqCst)
    }

    /// Transcribe an audio file
    pub fn transcribe(&self, audio_path: &Path) -> Result<TranscriptionResult, TranscriptionError> {
        if self.is_transcribing.swap(true, Ordering::SeqCst) {
            return Err(TranscriptionError::AlreadyTranscribing);
        }

        let result = self.transcribe_internal(audio_path);
        self.is_transcribing.store(false, Ordering::SeqCst);
        result
    }

    fn transcribe_internal(&self, audio_path: &Path) -> Result<TranscriptionResult, TranscriptionError> {
        if !audio_path.exists() {
            return Err(TranscriptionError::AudioNotFound(
                audio_path.to_string_lossy().to_string(),
            ));
        }

        // Read the WAV file and convert to f32 samples
        let samples = self.load_audio(audio_path)?;

        // Serialised against live transcription, which shares this context.
        let _inference = crate::transcription::lock_inference();

        // Create whisper state
        let mut state = self
            .ctx
            .create_state()
            .map_err(|e| TranscriptionError::TranscriptionFailed(e.to_string()))?;

        // Set up transcription parameters
        let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });

        // Configure for better meeting transcription
        params.set_language(Some("en")); // Default to English, can be made configurable
        params.set_translate(false);
        params.set_print_special(false);
        params.set_print_progress(false);
        params.set_print_realtime(false);
        params.set_print_timestamps(false);
        params.set_token_timestamps(true);
        params.set_n_threads(num_cpus());

        // Run the transcription
        state
            .full(params, &samples)
            .map_err(|e| TranscriptionError::TranscriptionFailed(e.to_string()))?;

        // Extract segments
        let num_segments = state.full_n_segments().map_err(|e| {
            TranscriptionError::TranscriptionFailed(e.to_string())
        })?;

        let mut segments = Vec::new();
        let mut full_text = String::new();

        for i in 0..num_segments {
            let start_time = state.full_get_segment_t0(i).map_err(|e| {
                TranscriptionError::TranscriptionFailed(e.to_string())
            })? as f64 / 100.0; // Convert centiseconds to seconds

            let end_time = state.full_get_segment_t1(i).map_err(|e| {
                TranscriptionError::TranscriptionFailed(e.to_string())
            })? as f64 / 100.0;

            let text = state.full_get_segment_text(i).map_err(|e| {
                TranscriptionError::TranscriptionFailed(e.to_string())
            })?;

            let text = text.trim().to_string();
            if !text.is_empty() {
                if !full_text.is_empty() {
                    full_text.push(' ');
                }
                full_text.push_str(&text);

                segments.push(TranscriptionSegment {
                    start_time,
                    end_time,
                    text,
                    // Local Whisper has no diarizer. Attribution comes from
                    // which track a segment was read out of, and is attached
                    // further up.
                    speaker: None,
                });
            }
        }

        Ok(TranscriptionResult {
            segments,
            full_text,
            language: Some("en".to_string()),
        })
    }

    /// Load audio file and convert to 16kHz mono f32 samples
    fn load_audio(&self, audio_path: &Path) -> Result<Vec<f32>, TranscriptionError> {
        let reader = hound::WavReader::open(audio_path)
            .map_err(|e| TranscriptionError::TranscriptionFailed(format!("Failed to open WAV: {}", e)))?;

        let spec = reader.spec();
        let sample_rate = spec.sample_rate;
        let channels = spec.channels as usize;

        // Read samples based on format
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => {
                reader
                    .into_samples::<f32>()
                    .filter_map(|s| s.ok())
                    .collect()
            }
            hound::SampleFormat::Int => {
                let bits = spec.bits_per_sample;
                let max_val = (1 << (bits - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .filter_map(|s| s.ok())
                    .map(|s| s as f32 / max_val)
                    .collect()
            }
        };

        // Convert to mono if stereo
        let mono_samples: Vec<f32> = if channels > 1 {
            samples
                .chunks(channels)
                .map(|chunk| chunk.iter().sum::<f32>() / channels as f32)
                .collect()
        } else {
            samples
        };

        // Resample to 16kHz if needed (Whisper requires 16kHz)
        let target_rate = 16000;
        let resampled = if sample_rate != target_rate {
            resample(&mono_samples, sample_rate, target_rate)
        } else {
            mono_samples
        };

        Ok(resampled)
    }
}

/// Get the number of CPU threads to use
fn num_cpus() -> i32 {
    std::thread::available_parallelism()
        .map(|n| n.get() as i32)
        .unwrap_or(4)
        .min(8) // Cap at 8 threads for transcription
}

/// Simple linear resampling (for basic use; a proper resampler would be better for production)
fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    let ratio = to_rate as f64 / from_rate as f64;
    let new_len = (samples.len() as f64 * ratio) as usize;
    let mut result = Vec::with_capacity(new_len);

    for i in 0..new_len {
        let src_idx = i as f64 / ratio;
        let idx0 = src_idx.floor() as usize;
        let idx1 = (idx0 + 1).min(samples.len() - 1);
        let frac = src_idx - idx0 as f64;

        let sample = samples[idx0] as f64 * (1.0 - frac) + samples[idx1] as f64 * frac;
        result.push(sample as f32);
    }

    result
}

