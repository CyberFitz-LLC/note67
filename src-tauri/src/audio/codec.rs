//! Reading and writing recorded audio.
//!
//! Recordings are stored as 16 kHz mono FLAC. Two separate decisions sit behind
//! that, and only the first one costs anything:
//!
//! **16 kHz mono** is what every consumer of this audio already asks for —
//! `transcriber.rs` and `converter.rs` both resample to exactly that before
//! doing anything. Storing 48 kHz stereo meant keeping five sixths of a signal
//! that nothing read, at roughly a gigabyte an hour across the two tracks. This
//! is the lossy step: the archive is now the transcription-grade recording
//! rather than the original capture.
//!
//! **FLAC** is then lossless on top of it, so nothing further is given up, and
//! it needs no C toolchain — `flacenc` is pure Rust and symphonia already
//! decodes FLAC. Opus would be several times smaller again and needs libopus
//! for both directions, which is a native dependency on the Windows build and
//! was judged not worth it for the extra ratio.
//!
//! Reads go through symphonia rather than `hound`, which is what keeps every
//! recording made before this change readable: the format is detected from the
//! file, so old WAVs and new FLACs both just work.

use std::fs::File;
use std::path::Path;

use flacenc::component::BitRepr;
use flacenc::error::Verify;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::audio::AudioError;

/// The rate everything downstream resamples to anyway.
pub const TARGET_RATE: u32 = 16_000;

/// A decoded recording: interleaved samples, plus how to read them.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub channels: u16,
}

impl Decoded {
    /// Fold to one channel, averaging rather than dropping.
    ///
    /// Dropping the extra channels would lose whatever was only on the right —
    /// which on a system-audio capture can be an entire participant.
    pub fn to_mono(&self) -> Vec<f32> {
        if self.channels <= 1 {
            return self.samples.clone();
        }
        let ch = self.channels as usize;
        self.samples
            .chunks(ch)
            .map(|frame| frame.iter().sum::<f32>() / ch as f32)
            .collect()
    }
}

/// Decode a recording, whatever container it is in.
///
/// Deliberately format-agnostic: recordings made before the move to FLAC are
/// WAV, and they have to keep playing and keep transcribing. Symphonia probes
/// the file rather than trusting the extension, so a mislabelled file works too.
pub fn decode(path: &Path) -> Result<Decoded, AudioError> {
    let file = File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());

    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }

    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| AudioError::IoError(std::io::Error::other(format!("unreadable audio: {e}"))))?;

    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        .ok_or_else(|| AudioError::IoError(std::io::Error::other("no audio track")))?
        .clone();

    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| AudioError::IoError(std::io::Error::other(format!("no decoder: {e}"))))?;

    let mut samples: Vec<f32> = Vec::new();
    let mut sample_rate = track.codec_params.sample_rate.unwrap_or(TARGET_RATE);
    let mut channels = track
        .codec_params
        .channels
        .map(|c| c.count() as u16)
        .unwrap_or(1);

    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            // The normal end of a stream arrives as an error from symphonia.
            Err(_) => break,
        };
        if packet.track_id() != track.id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(audio) => {
                let spec = *audio.spec();
                sample_rate = spec.rate;
                channels = spec.channels.count() as u16;
                let mut buf = SampleBuffer::<f32>::new(audio.capacity() as u64, spec);
                buf.copy_interleaved_ref(audio);
                samples.extend_from_slice(buf.samples());
            }
            // One bad packet should not cost the whole recording.
            Err(symphonia::core::errors::Error::DecodeError(_)) => continue,
            Err(e) => {
                return Err(AudioError::IoError(std::io::Error::other(format!(
                    "decode failed: {e}"
                ))))
            }
        }
    }

    Ok(Decoded {
        samples,
        sample_rate,
        channels,
    })
}

/// Linear resample. Same approach as `converter.rs`, which this matches
/// deliberately: two resamplers that round differently would put the transcript
/// and the playback track on slightly different clocks.
pub fn resample(samples: &[f32], from_rate: u32, to_rate: u32) -> Vec<f32> {
    if from_rate == to_rate || from_rate == 0 || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from_rate as f64 / to_rate as f64;
    let out_len = ((samples.len() as f64) / ratio).floor() as usize;
    (0..out_len)
        .map(|i| {
            let src = i as f64 * ratio;
            let lo = src.floor() as usize;
            let hi = (lo + 1).min(samples.len() - 1);
            let t = (src - lo as f64) as f32;
            samples[lo] * (1.0 - t) + samples[hi] * t
        })
        .collect()
}

/// Decode anything and hand back exactly what storage and transcription want.
pub fn decode_to_16k_mono(path: &Path) -> Result<Vec<f32>, AudioError> {
    let decoded = decode(path)?;
    let mono = decoded.to_mono();
    Ok(resample(&mono, decoded.sample_rate, TARGET_RATE))
}

/// Write mono 16 kHz samples as FLAC.
pub fn encode_flac_16k_mono(samples: &[f32], path: &Path) -> Result<(), AudioError> {
    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| AudioError::IoError(std::io::Error::other(format!("flac config: {e:?}"))))?;

    let mut pcm: Vec<i32> = samples
        .iter()
        // Rounded, not truncated. `as i32` truncates toward zero, which loses
        // up to a whole least-significant bit on every sample and biases the
        // signal toward silence; rounding halves the error and centres it.
        //
        // Clamped, because a sample outside [-1, 1] wraps rather than clips
        // when it is cast, turning the loudest moment into noise.
        .map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i32)
        .collect();

    // Padded up to a whole number of blocks, with silence.
    //
    // flacenc encodes a full block even when the final read is short, while
    // STREAMINFO records the true sample count — so for any length that is not
    // a multiple of the block size the frames and the declared total disagree,
    // and a decoder rejects the file outright ("end of stream"). Verified
    // across sizes: only exact multiples survive a round trip.
    //
    // Padding makes the two agree. It costs at most one block of trailing
    // silence — 4096 samples, a quarter-second at 16 kHz — at the very end of a
    // recording, where it changes nothing anyone hears or transcribes.
    let block = config.block_size;
    if !pcm.is_empty() {
        let remainder = pcm.len() % block;
        if remainder != 0 {
            pcm.resize(pcm.len() + (block - remainder), 0);
        }
    }

    let source = flacenc::source::MemSource::from_samples(&pcm, 1, 16, TARGET_RATE as usize);
    let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
        .map_err(|e| AudioError::IoError(std::io::Error::other(format!("flac encode: {e:?}"))))?;

    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| AudioError::IoError(std::io::Error::other(format!("flac write: {e:?}"))))?;

    std::fs::write(path, sink.as_slice())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A second of speech-ish signal: a couple of tones plus a quiet passage,
    /// so the encoder has something with structure rather than a pure sine it
    /// can model perfectly.
    fn signal(len: usize) -> Vec<f32> {
        (0..len)
            .map(|i| {
                let t = i as f32 / TARGET_RATE as f32;
                let env = if i % 4000 < 500 { 0.05 } else { 1.0 };
                env * (0.4
                    * (2.0 * std::f32::consts::PI * 220.0 * t).sin()
                    + 0.2 * (2.0 * std::f32::consts::PI * 880.0 * t).sin())
            })
            .collect()
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("note67-codec-{name}"));
        p
    }

    #[test]
    fn flac_round_trips_without_losing_the_signal() {
        // The whole claim of choosing FLAC is that the compression itself gives
        // nothing up. If this drifts, the claim is false.
        let samples = signal(TARGET_RATE as usize);
        let path = tmp("roundtrip.flac");
        encode_flac_16k_mono(&samples, &path).expect("encode");

        let back = decode_to_16k_mono(&path).expect("decode");
        assert!(
            back.len() >= samples.len(),
            "samples were lost: {} < {}",
            back.len(),
            samples.len()
        );
        // Anything past the original is block padding, and must be silent.
        assert!(
            back[samples.len()..].iter().all(|s| s.abs() < 1e-3),
            "the padding is not silent"
        );

        // Two 16-bit steps, and the reason for each is worth stating so this
        // bound is not quietly widened later:
        //
        //   one  — the f32 -> i16 quantisation on the way in, which the WAV
        //          path performed identically.
        //   one  — a scale convention. Full scale is written as 32767 and read
        //          back as a fraction of 32768, so the two differ by an LSB.
        //
        // Neither is the compression. FLAC itself gives up nothing, which is
        // the whole reason it was chosen over Opus.
        let step = 2.0 * (2.0 / u16::MAX as f32);
        for (i, (a, b)) in samples.iter().zip(back.iter()).enumerate() {
            assert!(
                (a - b).abs() <= step,
                "sample {i} moved by {} (more than one 16-bit step)",
                (a - b).abs()
            );
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn flac_is_substantially_smaller_than_the_wav_it_replaces() {
        // Not a precise ratio — that depends on content — but the feature is
        // pointless if it does not clearly beat PCM.
        let samples = signal(TARGET_RATE as usize * 3);
        let path = tmp("size.flac");
        encode_flac_16k_mono(&samples, &path).expect("encode");

        let flac = std::fs::metadata(&path).expect("stat").len();
        let pcm = (samples.len() * 2) as u64;
        assert!(
            flac < pcm,
            "FLAC ({flac}) should be smaller than raw PCM ({pcm})"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_wav_recording_made_before_this_change_still_decodes() {
        // The reason reads go through symphonia rather than hound. There are
        // gigabytes of these on disk and they have to keep working.
        let path = tmp("legacy.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: TARGET_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).expect("wav");
        for s in signal(8000) {
            w.write_sample((s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .expect("sample");
        }
        w.finalize().expect("finalize");

        let back = decode_to_16k_mono(&path).expect("decode a legacy WAV");
        assert_eq!(back.len(), 8000);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_stereo_48k_recording_becomes_mono_16k() {
        // The shape of every existing system-audio capture.
        let path = tmp("legacy-48k-stereo.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&path, spec).expect("wav");
        // Left carries the signal, right is silent — so a channel that was
        // dropped rather than averaged would show up as half amplitude.
        for s in signal(48_000) {
            let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
            w.write_sample(v).expect("l");
            w.write_sample(0i16).expect("r");
        }
        w.finalize().expect("finalize");

        let back = decode_to_16k_mono(&path).expect("decode");
        assert!(
            (back.len() as i64 - 16_000).abs() <= 1,
            "expected ~16000 samples, got {}",
            back.len()
        );
        assert!(
            back.iter().any(|s| s.abs() > 0.05),
            "the signal was lost in the fold to mono"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_length_that_is_not_a_whole_number_of_blocks_still_round_trips() {
        // The interop bug this guards: flacenc writes a full final block while
        // STREAMINFO records the true count, so an unpadded stream is rejected
        // outright. Every real recording lands on an arbitrary length, so
        // without the padding almost nothing would decode.
        for n in [1usize, 100, 4095, 4097, 16_000] {
            let path = tmp(&format!("odd-{n}.flac"));
            let s: Vec<f32> = (0..n).map(|i| ((i as f32) / 50.0).sin() * 0.3).collect();
            encode_flac_16k_mono(&s, &path).expect("encode");
            let back = decode_to_16k_mono(&path)
                .unwrap_or_else(|e| panic!("length {n} failed to decode: {e}"));
            assert!(back.len() >= n, "length {n} lost samples");
            assert!(
                back.len() - n < 4096,
                "length {n} padded by a whole block or more"
            );
            for (i, (a, b)) in s.iter().zip(back.iter()).enumerate() {
                assert!((a - b).abs() < 1e-3, "length {n}, sample {i} drifted");
            }
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn an_empty_recording_does_not_panic() {
        let path = tmp("empty.flac");
        encode_flac_16k_mono(&[], &path).expect("encode nothing");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn silence_survives_the_round_trip_as_silence() {
        let path = tmp("silence.flac");
        encode_flac_16k_mono(&vec![0.0; 4096], &path).expect("encode");
        let back = decode_to_16k_mono(&path).expect("decode");
        assert!(back.iter().all(|s| s.abs() < 1e-3), "silence gained noise");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn samples_beyond_full_scale_clip_rather_than_wrap() {
        // Without the clamp a loud moment casts past i16::MAX and wraps to a
        // large negative — a click, in the loudest part of the recording.
        let path = tmp("hot.flac");
        encode_flac_16k_mono(&[1.6, -1.6, 0.5], &path).expect("encode");
        let back = decode_to_16k_mono(&path).expect("decode");
        assert!(back[0] > 0.9, "positive peak wrapped: {}", back[0]);
        assert!(back[1] < -0.9, "negative peak wrapped: {}", back[1]);
        let _ = std::fs::remove_file(&path);
    }

}
