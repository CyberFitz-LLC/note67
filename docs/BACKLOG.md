# Backlog

Work that is real but not in flight. Newest thinking at the top of each section;
items move out of here into a branch or an issue when they start.

Design documents live in [`exochain/`](exochain/) and [`sync/`](sync/); this file
is only the list.

---

## Blocked on someone else

| Item | Blocked by |
|---|---|
| Endpoint → counterparty DID mapping, which turns evidence into enforcement | [exochained-toolkit#4](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/4) |
| Conformance suite and CI gate | toolkit phases P2–P3 are unbuilt |

### No longer blocked (corrected 2026-08-11)

Three entries here were wrong, and being wrong about them cost weeks of not
starting.

**Production is not blocked on the AVC root-trust loader.** Bob shipped the
boot-time loader on 2026-05-31 (deploy `58e9eb62-…`); the operational issuer
`did:exo:8EVGmqLo15…` registers from the verified bundle at startup, and both
waiting credentials returned `HTTP 200 {"status":"registered"}` when replayed.
The gap was real between 2026-05-30 and 2026-05-31 and has been closed since.
Production `/health` answers 200 today.

**[exochain#812](https://github.com/exochain/exochain/issues/812) never blocked
Note67.** It breaks `cargo add exochain-core` from crates.io, which matters to
external developers and to nobody in this stack. The AVC workspace has always
consumed the crates by **git rev at the ceremony commit**
`dd18b58cd49f9c96f396180bb72722db4f7d70d7`, and that path builds — as long as
two pre-release pins come with it, which is what `avc-app-registry`'s committed
lock encodes and a fresh resolve does not:

```toml
pkcs8 = "=0.11.0-rc.11"
spki  = "=0.8.0-rc.4"
```

Without them cargo takes released `pkcs8 0.11.0` / `spki 0.8.0`, and `ml-dsa
0.1.0-rc.7` fails against trait impls that exist only in the `-rc` line. Verified
from a clean two-dependency crate.

**The DID derivation does not need swapping.** Note67's implementation, written
against the spec because the crate would not build, produces a byte-identical
DID to `exo_identity::did::did_from_public_key` — verified against the pinned
all-zero-seed vector with both implementations compiled into one test. Adopting
the crate is now a dependency choice rather than a correctness fix.

**[toolkit#3](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/3)
has a defensible answer already.** `AvcSubjectKind::Service { service_id }` is
what the PRD picked, and it stays right until a Device variant exists. Bob's
answer would let it be reconsidered; it does not gate the build.

**[toolkit#2](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/2)
is answered — but not the way I first said.** I claimed the PRD settled it, on
the strength of the route it names. The PRD's route is not evidence: the node
answers `401` for *any* path under `/api/v1` before routing, including a
nonsense one, so a status code says nothing about what exists. The ceremony
commit the AVC crates are pinned to has **no emit route at all**; the deployed
node is newer.

Read out of `exo-node/src/avc.rs` on `main`, the contract is:

```
POST /api/v1/avc/receipts/emit
{
  "validation": {
    "credential": <the installed AVC, verbatim>,
    "action": {
      "action_id": <Hash256>,
      "actor_did": <this install's DID>,
      "requested_permission": "Write",
      "tool": "note67.meeting.attest",
      "data_class": "PersonalData",
      "requires_human_approval": false,
      "action_name": "note67.meeting.attest"
    },
    "now": <Timestamp>
  },
  "subject_signature": <Ed25519 over the payload below>,
  "subject_public_key": <this install's key>
}
```

Two things that are not obvious and would each cost a day:

- **What is signed** is `avc_action_signature_payload(&credential, &action, &now)`
  from `exo-avc` — not the action alone. Reimplementing it would be exactly the
  divergence the shared-crate rule exists to prevent, so the app should take the
  crate. Verified 2026-08-13 that `exo-avc` at the ceremony commit resolves and
  compiles inside the Tauri dependency tree, given the two pre-release pins.
- **`subject_public_key` is what routes around
  [exochain#687](https://github.com/exochain/exochain/issues/687).** The node
  prefers a registered key and falls back to the supplied one after checking it
  derives to `actor_did`. Both existing emitters supply it; without it a
  correctly registered credential still returns `401 subject public key is
  unresolved`.

Read back with `GET /api/v1/avc/receipts/<hash>`, which is auth-gated and needs
the node's admin token.

## Unverified on real hardware

Everything here compiles and passes tests; none of it has been exercised against
the physical thing it controls.

- **Microphone picker** — switch between two mics, record, confirm the audio
  follows the choice.
- **Speaker picker** (Windows loopback) — `windows.rs` has only ever been
  compiled by CI, never run.
- **Inference serialization fix** — the hypothesis for the crash ten seconds
  into a recording. If it crashes again there, the concurrency diagnosis was
  wrong and the Event Viewer exception code is what decides.
- ~~**Remote model endpoints**~~ — done 2026-08-20, and it found a real defect.
  The Spark's vLLM serves `lightning-30b-nvfp4` with `--reasoning-parser
  nemotron_v3`, whose delimiter the model never emits, so every reply came back
  `content: null` with the answer in `reasoning`. Summaries saved empty and task
  generation found nothing. The client now falls back to `reasoning` only when
  `content` is empty, and an empty reply is an error rather than a saved
  summary. **The server is still misconfigured** — every OpenAI-compatible
  client on :8902 sees null content, so LiteLLM and Fitzy are affected too.

## Streaming recogniser (shipped 2026-08-20, incomplete)

The third backend: live audio streamed to the Spark's Nemotron ASR over two
websockets, one per track. Demoed successfully. What is not done:

- **No echo suppression.** The local path runs `is_echo_of_system` so the room's
  speakers playing the far end does not get transcribed twice. The streaming
  path has no equivalent, so on speakers every remote utterance appears twice —
  once as `Others` from the system track and once as `You` from the microphone
  hearing it. Headphones hide this entirely, which is why the demo looked clean.
  The existing filter compares against a rolling window of system segments and
  the streaming path's timing differs enough that it needs testing rather than
  copying.
- **Untested over a real meeting length.** Longest verified run is a few
  seconds. Memory growth, timeouts, reconnect behaviour and what arrives during
  long silences are all unknown — flagged by the Spark-side brief and still
  open.
- **No diarization on this path.** The upload backend returns `Speaker 1..N`;
  this returns only the two track labels.
- **The recogniser is unauthenticated** and bound `0.0.0.0` on VLAN3. Fine on
  the LAN, and the settings copy says so, but it is not a thing to expose.

## Audio routing and speaker attribution

The three of these compound: with one mixed track and no diarization, a
ten-person call transcribes as two speakers, and the interesting half is
missing. Ordered by what unblocks what.

### Device test panel — in progress

Two real defects found 2026-08-12, both matching what a live meeting showed:

1. **The level meter only ever measured the microphone.** `process_audio` in
   `recorder.rs` is the only writer of `audio_level`, and it is the *input*
   stream's callback. The Windows loopback path never touches it. So changing
   the system-audio device could not move the meter — the meter was not
   measuring that track.
2. **The device is bound when the stream opens.** `run_recording` reads the
   preference once and calls `open_input_device`. Changing it mid-recording
   writes to a mutex nothing re-reads until the next recording, so the change
   silently does nothing until then.

`audio/levels.rs` is the measurement half: RMS, held peak, and a verdict
(silent / quiet / healthy / clipping). Peak is held because a meter polled a few
times a second misses the transients that matter, and clipping is judged from
peak because a clipping track can sit at a perfectly ordinary average — which is
how it goes unnoticed until the transcript is bad.

Still to build: a per-track meter for the system capture, a test that runs
outside a recording, and a panel that states the pass condition rather than just
showing bars. The condition worth stating is **your own voice must not appear on
the system track** — a mixed track is the failure that produces a light
transcript, and it looks fine on a meter.

The test must drive the same code path as recording. One that opened its own
streams could pass while recording failed.

### Merging transcripts of the same meeting

Today an import always creates a new note, because merging would interleave
content Note67 produced with content it did not, and a version records a single
origin. That is right for one transcript and wrong for the real case: the same
meeting recorded by Note67, Teams and Otter at once.

Three sources of the same hour are worth more than three separate notes,
because **where they disagree is information**. Teams knows who was speaking
because it has each participant on a separate stream; Note67 has better audio
of the room; Otter has its own errors. Aligning them gives per-segment
attribution the local recording cannot produce alone.

Design questions this raises, none of them settled:

- **Alignment.** Clocks differ, and recordings start at different moments. The
  offset has to be recovered from the content — a coarse text alignment over
  timestamps, not timestamps alone.
- **Provenance per segment.** A merged transcript is not `Recorded` or
  `Imported`; it is derived from several sources with different standing. The
  chain records one origin per version, so a merged version needs either a new
  origin or a per-segment source. That decision bounds what a receipt over it
  may claim, so it is not cosmetic.
- **Which text wins.** Probably the highest-confidence source per segment, but
  "confidence" is not comparable across tools.
- **Idempotence.** Importing the same Teams file twice must not double the
  transcript.

### Speaker identification

Wanted: `Speaker 1..n` instead of "You" and "Others", relabelling to real names,
suggestions once someone has been labelled enough times, and always a manual
override for a specific stretch.

This is diarization, and it is a genuinely different thing from transcription —
whisper.cpp does not do it. Realistic options, in order of cost:

1. **Track-derived attribution**, which is what exists: which file a segment came
   from. Correct for "me versus everyone else" and useless beyond it.
2. **A diarization model** (pyannote-class, or whisper.cpp's tinydiarize for two
   speakers). Real dependency, real GPU time, offline. **Partly built:**
   `note67-asr` on the Spark (Parakeet TDT + NeMo ClusteringDiarizer) returns
   `Speaker 1..N` and is wired into the *upload* path. Its accuracy is still
   unestablished — 4 speakers found in a synthetic 6-voice fixture, and espeak
   voices are a poor proxy. **A real meeting recording is the missing input.**
3. **Speaker embeddings + a labelled store.** Embed each speaker turn, cluster
   within a meeting for `Speaker N`, and match against previously labelled
   embeddings across meetings to suggest names. This is the version that
   remembers people.

Notes on doing it properly:

- **Voice prints are biometric data.** Storing embeddings of named colleagues is
  a different privacy claim from storing a transcript, and the credential's
  `data_classes` and the consent wording both have to say so.
- **A manual label must always win** and must survive re-transcription, so labels
  belong on a stable segment identity rather than on an index into the text.
- **Relabelling changes the transcript**, so it appends a chain version. Attaching
  a name is an edit to the content, and the chain would otherwise have a blind
  spot exactly where the most human-meaningful change happens.
- Merging (above) is the cheapest large win here: Teams already knows the names.

## Note67 app

- **Manual transcript edits do not create a version.** `Reason::Edit` exists and
  nothing produces it, because the app has no transcript editing surface. When
  one is added, it must append a version or the chain will have a blind spot
  exactly where tampering would occur.
- **Attachments and embedded images do not sync.** Note bodies can embed images;
  those are files rather than rows and need object storage. Excluded from the
  first sync release deliberately — see `sync/DESIGN.md` §10.
- **Speaker attribution depends on track separation.** With VoiceMeeter feeding
  a full mix into the microphone input, every speaker lands on the mic track and
  is labelled "You". There is no diarization; attribution is purely which file a
  segment came from. Either routing guidance or real diarization would fix it.
- **Chunking is tuned for a 4096 context.** `MAX_CONTENT_LENGTH` and
  `split_into_chunks` predate configurable providers, so a long-context model
  gets more round-trips than it needs. Works, wastes time.
- **Recordings are never pruned.** Uncompressed WAV at roughly 10 MB per minute
  per track, two tracks per meeting, nothing cleans up.
- **DOCX transcript import.** VTT covers Teams; DOCX is the other export and
  yields poorer structure.

## Sync service

Design in [`sync/DESIGN.md`](sync/DESIGN.md); code in
[CyberFitz-LLC/note67-sync](https://github.com/CyberFitz-LLC/note67-sync).

1. ~~Extract the canonical transcript form into a shared crate~~ — done:
   `note67-canonical`, consumed by the service as a git dependency, so the hash
   is computed by one implementation rather than two that agree today.
2. ~~Service skeleton: axum, Entra JWT validation, device registration by DID.~~
3. ~~Sync protocol: change feed, per-device cursors, tombstones, chain re-base.~~
   Tested against a real Postgres; nothing deployed, and no live Entra token has
   ever reached it.
4. Sharing: owner and explicit shares, Graph-resolved group membership. Until
   this exists a note is visible only to whoever created it — `access.rs` has
   the shape but `Owner` is the only role it can return.
5. Client: sign-in, background sync, "Remove from this device" versus "Delete
   from archive".
6. Receipts for mirroring and for sharing.
7. Retention policy support — never forgets by default.

Smaller things the protocol left behind:

- **A feed page carries whole transcripts.** Each version travels with every
  segment it hashes, so `MAX_LIMIT` is set for the heaviest kind rather than the
  average one. If pages get unwieldy, versions need a separate content fetch.
- **Tombstones and `applied_changes` grow without bound.** Correct — the archive
  never forgets by default — but the idempotency table has no reason to keep
  rows past the point any client could still retry them.
- **Deleting from the archive destroys the chain for that note.** The honest
  reading of "delete", and a real loss of evidence. If that trade ever needs
  revisiting, it is here.

## Build and release

- **CUDA CI build has never succeeded.** It builds locally. On a hosted runner
  `whisper-rs-sys`'s CMake step fails identifying the CUDA compiler
  (`CMakeCUDACompilerId.cu`), which is the same Visual Studio integration gap
  the workflow's copy step was meant to close — and evidently does not, at
  least for compiler identification rather than only for the build. The Vulkan
  job is green, so **CI does produce a working installer**; only the CUDA
  variant is missing, and a local `build-windows-gpu.ps1` covers that.
- **The NSIS bundler fetches its toolchain at build time.** A Vulkan build
  failed with `failed to bundle project: io: Peer disconnected` and succeeded
  on a straight re-run. Worth a retry before believing a bundling failure —
  and worth pinning if it recurs, because a network blip currently looks like
  a build error.
- **Line endings are pinned in `.gitattributes`** (added 2026-08-12). Before
  that, a Windows checkout could show a file as modified with an *empty*
  `git diff`, which made `git checkout <branch>` abort — and the build then
  ran from stale code and produced a normal-looking installer missing a week
  of work. If a checkout ever refuses again, `git log --oneline -1` before
  building is what catches it.
- **`release.yml` needs two fixes before a real release.** It creates draft
  releases, and `/releases/latest/download/` only resolves for published ones, so
  the updater sees nothing until the draft is published by hand. Its macOS jobs
  will also fail in this fork for want of Apple signing secrets.
- **Local Windows builds need six things CI has preinstalled.** Documented in
  `scripts/build-windows-gpu.ps1`, which now checks for each — but the list is a
  reason to prefer CI once Actions is healthy.
- **Upstream merge tracking.** This fork diverges from `ZapYap-com/note67`;
  byte-parity and scope drift are the usual breakers when taking a merge.

## Questions worth asking Bob

- Was LYNK designed with a desktop meeting recorder in mind? It fits almost
  exactly, and if that was deliberate he may have opinions the repos do not
  record.
- The claims-vs-code audit (exochain #696–700) found the receipt path does not
  support "legally admissible". Meeting receipts deliberately claim less, but
  that gap should close before anyone markets the stronger version.
