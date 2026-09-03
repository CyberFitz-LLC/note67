//! Live transcription by a streaming recogniser.
//!
//! The third backend. `Local` is live but limited to on-device Whisper;
//! `Remote` is stronger but only works on a finished recording. This is live
//! *and* stronger — at a privacy cost higher than either, because raw audio
//! leaves the machine continuously while a meeting is in progress rather than
//! one file being uploaded after the fact. See `docs/exochain/DECISIONS.md`.
//!
//! **Two sockets, not one.** Live capture keeps the microphone and the system
//! output as separate tracks, and that separation is the only speaker
//! attribution the app has without a diarizer: mic is "You", system is
//! "Others". Mixing them into one stream to save a connection would throw that
//! away — and this recogniser does not diarize, so nothing would give it back.

use serde::Deserialize;

/// Chunk length on the wire. The reference client paces ~100 ms frames in real
/// time, and this is a streaming recogniser rather than a file endpoint: firing
/// audio as fast as the socket accepts is not the tested path.
pub const CHUNK_MS: usize = 100;
pub const SAMPLE_RATE: usize = 16_000;

/// Samples per frame, one channel.
pub const CHUNK_SAMPLES: usize = SAMPLE_RATE * CHUNK_MS / 1000;

/// What the service sends back.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerEvent {
    /// Sent once on connect.
    Ready,
    Transcript {
        #[serde(default)]
        text: String,
        #[serde(default)]
        is_final: bool,
    },
    /// Anything this client has not been taught. Ignored rather than treated as
    /// an error: a new event type is far more likely than a broken service, and
    /// tearing down a live meeting's connection over one would lose audio that
    /// cannot be recovered.
    #[serde(other)]
    Unknown,
}

/// Parse a text frame.
///
/// A frame that will not parse is `Unknown` rather than an error, for the same
/// reason: nothing about a malformed status message justifies dropping the
/// stream a meeting is being recorded into.
pub fn parse_event(frame: &str) -> ServerEvent {
    serde_json::from_str(frame).unwrap_or(ServerEvent::Unknown)
}

/// The frame that ends an utterance. The final arrives after it, so the reader
/// has to keep going briefly rather than closing on send.
pub fn finalize_frame() -> String {
    r#"{"type":"reset","finalize":true}"#.to_string()
}

/// Convert captured audio to what the wire wants: 16 kHz mono s16le.
///
/// Run this *after* `normalize_peak`, so the recogniser hears the same levels
/// Whisper would have. Values are clamped before scaling because a normalised
/// buffer can still exceed 1.0 slightly, and wrapping that into i16 turns a
/// loud syllable into a burst of noise.
pub fn to_s16le(samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        let clamped = s.clamp(-1.0, 1.0);
        let v = (clamped * i16::MAX as f32) as i16;
        out.extend_from_slice(&v.to_le_bytes());
    }
    out
}

/// Where a track has got to, in seconds of audio sent.
///
/// The service's transcript events carry `text` and `is_final` and **no timing
/// at all** — verified against the running service, not assumed. So the only
/// clock is how much audio this socket has been given, which is why each track
/// keeps its own: two sockets are fed independently and their positions drift
/// apart the moment one of them is gated for silence.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackClock {
    pub samples_sent: usize,
}

impl TrackClock {
    pub fn advance(&mut self, samples: usize) {
        self.samples_sent += samples;
    }

    pub fn seconds(&self) -> f64 {
        self.samples_sent as f64 / SAMPLE_RATE as f64
    }

    /// The span a final covers: from where the last one ended to here.
    pub fn span_since(&self, previous: &TrackClock) -> (f64, f64) {
        (previous.seconds(), self.seconds())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ready_frame_parses() {
        assert_eq!(parse_event(r#"{"type":"ready"}"#), ServerEvent::Ready);
    }

    #[test]
    fn a_partial_and_a_final_are_distinguished() {
        // The whole UI behaviour rests on this: partials are redrawn, finals
        // are persisted.
        assert_eq!(
            parse_event(r#"{"type":"transcript","text":"hello","is_final":false}"#),
            ServerEvent::Transcript { text: "hello".into(), is_final: false }
        );
        assert_eq!(
            parse_event(r#"{"type":"transcript","text":"hello","is_final":true}"#),
            ServerEvent::Transcript { text: "hello".into(), is_final: true }
        );
    }

    #[test]
    fn the_real_final_frame_parses() {
        // Captured from the running service: it carries a `finalize` field the
        // brief did not mention, and an unknown field must not break parsing.
        assert_eq!(
            parse_event(r#"{"type":"transcript","text":"","is_final":true,"finalize":true}"#),
            ServerEvent::Transcript { text: String::new(), is_final: true }
        );
    }

    #[test]
    fn an_unknown_event_is_ignored_rather_than_fatal() {
        // Tearing down the connection a live meeting is recording into, over a
        // status message, would lose audio nobody can get back.
        assert_eq!(parse_event(r#"{"type":"something_new"}"#), ServerEvent::Unknown);
        assert_eq!(parse_event("not json at all"), ServerEvent::Unknown);
        assert_eq!(parse_event(""), ServerEvent::Unknown);
    }

    #[test]
    fn silence_converts_to_silence() {
        assert_eq!(to_s16le(&[0.0, 0.0]), vec![0, 0, 0, 0]);
    }

    #[test]
    fn conversion_is_little_endian_two_bytes_per_sample() {
        assert_eq!(to_s16le(&[1.0]).len(), 2);
        // 1.0 -> i16::MAX = 0x7FFF, low byte first.
        assert_eq!(to_s16le(&[1.0]), vec![0xFF, 0x7F]);
    }

    #[test]
    fn out_of_range_samples_clamp_instead_of_wrapping() {
        // A normalised buffer can still exceed 1.0, and wrapping would turn a
        // loud syllable into a burst of noise the recogniser reads as garbage.
        assert_eq!(to_s16le(&[2.0]), to_s16le(&[1.0]));
        assert_eq!(to_s16le(&[-2.0]), to_s16le(&[-1.0]));
    }

    #[test]
    fn a_chunk_is_a_tenth_of_a_second() {
        assert_eq!(CHUNK_SAMPLES, 1600);
        assert_eq!(to_s16le(&vec![0.0; CHUNK_SAMPLES]).len(), 3200);
    }

    #[test]
    fn a_clock_counts_the_audio_it_was_given() {
        let mut c = TrackClock::default();
        c.advance(SAMPLE_RATE);
        assert_eq!(c.seconds(), 1.0);
        c.advance(SAMPLE_RATE / 2);
        assert_eq!(c.seconds(), 1.5);
    }

    #[test]
    fn a_span_runs_from_the_previous_final_to_now() {
        let mut previous = TrackClock::default();
        previous.advance(SAMPLE_RATE * 2);
        let mut now = previous;
        now.advance(SAMPLE_RATE * 3);
        assert_eq!(now.span_since(&previous), (2.0, 5.0));
    }

    #[test]
    fn two_tracks_keep_independent_clocks() {
        // The reason a clock is per-track rather than global: the mic is gated
        // for silence independently of system audio, so the two positions drift
        // apart immediately. One shared clock would timestamp every segment on
        // whichever track happened to be busier.
        let mut mic = TrackClock::default();
        let mut system = TrackClock::default();
        mic.advance(SAMPLE_RATE);
        system.advance(SAMPLE_RATE * 4);
        assert_ne!(mic.seconds(), system.seconds());
    }
}

// ---------------------------------------------------------------------------
// The socket
// ---------------------------------------------------------------------------

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;

/// What a session hands back as it recognises.
#[derive(Debug, Clone, PartialEq)]
pub enum Recognised {
    /// Redrawn in place; never persisted.
    ///
    /// Carries a span for the same reason a final does: the UI groups
    /// same-speaker turns under one timestamp, and a partial pinned at 0:00
    /// would break out of the group it belongs to and then jump when the final
    /// replaced it.
    Partial {
        text: String,
        start_time: f64,
        end_time: f64,
    },
    /// Persisted, with the span it covers on this track.
    Final {
        text: String,
        start_time: f64,
        end_time: f64,
    },
    /// The socket is gone. The caller must stop recording into it rather than
    /// carry on with no recogniser attached — a meeting that appears to be
    /// recording and is not is the worst outcome this feature can produce.
    Disconnected { reason: String },
}

/// One track's connection.
///
/// Audio goes in through `send`, recognitions come out through the receiver
/// returned by `connect`. Dropping the session closes the socket.
pub struct StreamingSession {
    outgoing: mpsc::Sender<OutFrame>,
    alive: Arc<AtomicBool>,
}

/// What goes down the socket.
///
/// Audio, or a request to close the current utterance. Finalize has to travel
/// the same queue rather than being sent directly, or it would overtake audio
/// still waiting to go and cut an utterance short of its own last words.
#[derive(Debug)]
enum OutFrame {
    Audio(Vec<u8>),
    Finalize,
}

impl StreamingSession {
    /// Whether the socket is still up.
    ///
    /// Checked by the capture loop before each send, so a dead connection stops
    /// the recording rather than being discovered when someone reads the
    /// transcript afterwards.
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::SeqCst)
    }

    /// Queue a chunk.
    ///
    /// Never blocks the capture loop: audio arrives on a real-time thread, and
    /// waiting on a network send there would drop samples.
    ///
    /// The two ways this can fail are not the same thing and must not be
    /// treated as one. A dead socket means stop; a full queue means the
    /// recogniser is behind and audio is being lost. Returning a plain `false`
    /// for both made back-pressure look like a disconnect, and the only thing
    /// it produced was a log line claiming the recogniser had "stopped
    /// accepting audio" while it was simply overloaded.
    pub fn send(&self, pcm: Vec<u8>) -> SendOutcome {
        self.enqueue(OutFrame::Audio(pcm))
    }

    /// Close the current utterance.
    ///
    /// This recogniser returns a final **only** when asked, so without this a
    /// meeting is one utterance: partials that are ever-growing prefixes of the
    /// whole conversation, a single final at the end, and every timestamp at
    /// zero because nothing ever advanced. It also grows the recogniser's own
    /// working state without bound, which is why a transcript that starts crisp
    /// falls minutes behind after half an hour.
    pub fn finalize(&self) -> SendOutcome {
        self.enqueue(OutFrame::Finalize)
    }

    fn enqueue(&self, frame: OutFrame) -> SendOutcome {
        if !self.is_alive() {
            return SendOutcome::Disconnected;
        }
        match self.outgoing.try_send(frame) {
            Ok(()) => SendOutcome::Sent,
            Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => SendOutcome::Behind,
            Err(_) => SendOutcome::Disconnected,
        }
    }
}

/// How long to wait after asking for a final before closing the socket.
///
/// The recogniser answers a finalize asynchronously, so closing straight away
/// discards whatever was still being decoded — which is always the end of the
/// meeting, the part people go back to.
pub const FINALIZE_GRACE: Duration = Duration::from_millis(1500);

/// What happened to a chunk handed to a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendOutcome {
    Sent,
    /// The recogniser is not keeping up and this audio was dropped. The
    /// recording continues; the transcript will have a hole.
    Behind,
    /// The socket is gone. The session has to stop.
    Disconnected,
}

/// How many chunks may queue before the sender gives up.
///
/// Three seconds. Enough to ride out a stall; short enough that a recogniser
/// which has stopped keeping up is noticed while the meeting is still running.
pub const QUEUE_CHUNKS: usize = 30;

/// Open a session for one track.
pub async fn connect(
    ws_url: &str,
    label: &'static str,
) -> Result<(StreamingSession, mpsc::Receiver<Recognised>), String> {
    let (socket, _) = tokio_tungstenite::connect_async(ws_url)
        .await
        .map_err(|e| format!("could not open a {label} stream: {e}"))?;
    let (mut write, mut read) = socket.split();

    let (audio_tx, mut audio_rx) = mpsc::channel::<OutFrame>(QUEUE_CHUNKS);
    let (out_tx, out_rx) = mpsc::channel::<Recognised>(64);
    let alive = Arc::new(AtomicBool::new(true));

    // This track's clock, shared between the writer that advances it and the
    // reader that stamps finals with it. Per session, never global: two tracks
    // are gated for silence independently, so a shared counter would timestamp
    // every segment on whichever track happened to be busier.
    let sent = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    // Writer: audio out.
    let writer_alive = Arc::clone(&alive);
    let writer_sent = Arc::clone(&sent);
    tokio::spawn(async move {
        while let Some(frame) = audio_rx.recv().await {
            match frame {
                OutFrame::Audio(pcm) => {
                    // Two bytes per sample, s16le.
                    let samples = pcm.len() / 2;
                    if write
                        .send(tokio_tungstenite::tungstenite::Message::Binary(pcm))
                        .await
                        .is_err()
                    {
                        break;
                    }
                    // Advanced only after the send succeeds, so the clock never
                    // claims audio the recogniser was not given.
                    writer_sent.fetch_add(samples, Ordering::SeqCst);
                }
                OutFrame::Finalize => {
                    if write
                        .send(tokio_tungstenite::tungstenite::Message::Text(
                            finalize_frame(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }
        }
        // Ask for the last utterance before going. A final that never arrives
        // is speech the user said and the transcript will not contain.
        let _ = write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                finalize_frame(),
            ))
            .await;

        // Then give that final time to come back, and only then close.
        //
        // Both halves matter. Without the wait, the close races the last
        // utterance and truncates the end of the meeting. Without the close,
        // the server holds the connection open and the reader below never
        // returns — the socket and its task leak for the life of the app, one
        // pair per recording. The first version of this did exactly that and
        // hung a live test for ten minutes.
        tokio::time::sleep(FINALIZE_GRACE).await;
        let _ = write
            .send(tokio_tungstenite::tungstenite::Message::Close(None))
            .await;
        let _ = write.close().await;
        writer_alive.store(false, Ordering::SeqCst);
    });

    // Reader: recognitions in, with this track's own clock.
    let reader_alive = Arc::clone(&alive);
    let reader_sent = Arc::clone(&sent);
    tokio::spawn(async move {
        let mut clock = TrackClock::default();
        let mut last_final = TrackClock::default();

        while let Some(message) = read.next().await {
            let Ok(message) = message else { break };
            let frame = match message {
                tokio_tungstenite::tungstenite::Message::Text(t) => t,
                // Binary and control frames carry nothing this client reads.
                _ => continue,
            };

            match parse_event(&frame) {
                ServerEvent::Transcript { text, is_final } if is_final => {
                    // Empty finals are ordinary — silence, or the finalize that
                    // ends a session — and persisting them would litter the
                    // transcript with blank segments.
                    if !text.trim().is_empty() {
                        let (start_time, end_time) = clock.span_since(&last_final);
                        if out_tx
                            .send(Recognised::Final {
                                text: text.trim().to_string(),
                                start_time,
                                end_time,
                            })
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                    last_final = clock;
                }
                ServerEvent::Transcript { text, .. } => {
                    let (start_time, end_time) = clock.span_since(&last_final);
                    if out_tx
                        .send(Recognised::Partial {
                            text,
                            start_time,
                            end_time,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                ServerEvent::Ready | ServerEvent::Unknown => {}
            }

            // Read from the writer's counter rather than tracked here: the
            // reader never sees the audio, only what came back from it.
            clock.samples_sent = reader_sent.load(Ordering::SeqCst);
        }

        reader_alive.store(false, Ordering::SeqCst);
        let _ = out_tx
            .send(Recognised::Disconnected {
                reason: format!("the {label} stream closed"),
            })
            .await;
    });

    Ok((
        StreamingSession {
            outgoing: audio_tx,
            alive,
        },
        out_rx,
    ))
}

/// Tests that open a real socket.
///
/// Separate from the protocol tests above, which are pure. One of these stands
/// up a fake recogniser in-process and runs in the ordinary suite; the other is
/// `#[ignore]`d and needs the actual service.
#[cfg(test)]
mod socket_tests {
    use super::*;

    /// Exercises the real client against a real recogniser, on two concurrent
    /// sockets — the shape this backend actually runs in.
    ///
    /// Ignored by default: it needs the service and two speech clips. The
    /// clips are deliberately different sentences, so a crossed stream shows up
    /// as a failed assertion rather than as plausible-looking text.
    ///
    ///   espeak-ng -w a.wav "The access code is seven four two nine."
    ///   espeak-ng -w b.wav "Please send the quarterly report on Monday morning."
    ///   ffmpeg -i a.wav -ar 16000 -ac 1 -f s16le a.pcm      # and b
    ///
    ///   NOTE67_ASR_URL=ws://192.168.32.223:8080 \
    ///   NOTE67_ASR_CLIP_A=a.pcm NOTE67_ASR_CLIP_B=b.pcm \
    ///   cargo test --lib two_concurrent_sessions -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "needs a live recogniser and speech clips"]
    async fn two_concurrent_sessions_transcribe_without_crossing() {
        fn pcm_to_f32(bytes: &[u8]) -> Vec<f32> {
            bytes
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / i16::MAX as f32)
                .collect()
        }

        async fn play(url: String, label: &'static str, path: String) -> String {
            let samples = pcm_to_f32(&std::fs::read(&path).expect("clip"));
            let (session, mut rx) = connect(&url, label).await.expect("connect");

            tokio::spawn(async move {
                for frame in samples.chunks(CHUNK_SAMPLES) {
                    if session.send(to_s16le(frame)) == SendOutcome::Disconnected {
                        break;
                    }
                    // Paced like real time: firing a backlog at a streaming
                    // recogniser is not the path this runs in.
                    tokio::time::sleep(std::time::Duration::from_millis(
                        CHUNK_MS as u64,
                    ))
                    .await;
                }
                // Dropping closes the socket, via the finalize-then-close the
                // writer performs on its way out.
                drop(session);
            });

            let mut finals: Vec<String> = Vec::new();
            let mut partials = 0usize;
            // A backstop, not a timing assumption: if the session ever stops
            // ending itself the test says so in seconds instead of hanging.
            let deadline = std::time::Duration::from_secs(60);
            while let Ok(Some(item)) = tokio::time::timeout(deadline, rx.recv()).await {
                match item {
                    Recognised::Partial { .. } => partials += 1,
                    Recognised::Final {
                        text,
                        start_time,
                        end_time,
                    } => {
                        println!("[{label}] final {start_time:.2}-{end_time:.2}s  {text}");
                        finals.push(text);
                    }
                    Recognised::Disconnected { reason } => {
                        println!("[{label}] {reason} after {partials} partial(s)");
                        break;
                    }
                }
            }
            assert!(partials > 0, "{label} saw no partials");
            finals.join(" ").to_lowercase()
        }

        let url = std::env::var("NOTE67_ASR_URL")
            .unwrap_or_else(|_| "ws://192.168.32.223:8080".into());
        let a = std::env::var("NOTE67_ASR_CLIP_A").expect("NOTE67_ASR_CLIP_A");
        let b = std::env::var("NOTE67_ASR_CLIP_B").expect("NOTE67_ASR_CLIP_B");

        let (mic, system) = tokio::join!(
            play(url.clone(), "mic", a),
            play(url, "system", b),
        );

        println!("mic    : {mic}");
        println!("system : {system}");
        assert!(mic.contains("access code"), "mic transcript wrong: {mic}");
        assert!(
            system.contains("quarterly"),
            "system transcript wrong: {system}"
        );
        // The failure this is really for: two sockets served by one session on
        // the far side, so both tracks come back with the same words.
        assert!(!mic.contains("quarterly"), "the tracks crossed: {mic}");
        assert!(!system.contains("access code"), "the tracks crossed: {system}");
    }

    /// A recogniser that dies mid-recording must not take finished work with
    /// it, and must not leave the caller streaming into a socket that is gone.
    ///
    /// Uses a fake recogniser in-process rather than the real one, because the
    /// interesting moment — the far end vanishing part-way through a meeting —
    /// is not something you can ask a working service to do.
    #[tokio::test]
    async fn a_socket_that_dies_keeps_its_finals_and_reports_the_loss() {
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("handshake");
            use tokio_tungstenite::tungstenite::Message;
            let _ = ws.send(Message::Text(r#"{"type":"ready"}"#.into())).await;
            let _ = ws
                .send(Message::Text(
                    r#"{"type":"transcript","text":"we agreed on Thursday","is_final":true}"#
                        .into(),
                ))
                .await;
            // And now the service goes away, mid-recording, without a close
            // handshake — a container restart, not a polite shutdown.
            drop(ws);
        });

        let (session, mut rx) = connect(&format!("ws://{addr}"), "test")
            .await
            .expect("connect");

        let mut finals = Vec::new();
        let mut lost = None;
        while let Ok(Some(item)) =
            tokio::time::timeout(Duration::from_secs(5), rx.recv()).await
        {
            match item {
                Recognised::Final { text, .. } => finals.push(text),
                Recognised::Disconnected { reason } => {
                    lost = Some(reason);
                    break;
                }
                Recognised::Partial { .. } => {}
            }
        }

        assert_eq!(
            finals,
            vec!["we agreed on Thursday".to_string()],
            "a final that arrived before the socket died must still be delivered"
        );
        assert!(
            lost.is_some(),
            "the caller has to be told, or it carries on recording into nothing"
        );

        // And the session reports itself dead, which is what stops the capture
        // loop feeding audio nowhere.
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!session.is_alive());
        assert_eq!(
            session.send(vec![0, 0]),
            SendOutcome::Disconnected,
            "a dead session must report disconnection, not back-pressure"
        );
    }
    #[tokio::test]
    async fn a_recogniser_that_cannot_keep_up_reads_as_behind_not_as_gone() {
        // The distinction this exists for: under back-pressure the session is
        // perfectly alive, and treating a full queue as a disconnect made a
        // slow recogniser look like a broken one — while audio was quietly
        // dropped and the transcript fell minutes behind.
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");

        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("handshake");
            // Accepts the connection and then reads nothing, so the client's
            // queue fills — a recogniser that is up but overloaded.
            let _ = ws.send(tokio_tungstenite::tungstenite::Message::Text(
                r#"{"type":"ready"}"#.into(),
            ));
            tokio::time::sleep(Duration::from_secs(30)).await;
        });

        let (session, _rx) = connect(&format!("ws://{addr}"), "test")
            .await
            .expect("connect");

        let chunk = vec![0u8; CHUNK_SAMPLES * 2];
        let mut behind = 0;
        for _ in 0..(QUEUE_CHUNKS * 20) {
            match session.send(chunk.clone()) {
                SendOutcome::Behind => behind += 1,
                SendOutcome::Disconnected => {
                    panic!("back-pressure was reported as a disconnection")
                }
                SendOutcome::Sent => {}
            }
        }

        assert!(behind > 0, "the queue never filled, so nothing was tested");
        assert!(
            session.is_alive(),
            "the session must stay alive while merely overloaded"
        );
    }

}
