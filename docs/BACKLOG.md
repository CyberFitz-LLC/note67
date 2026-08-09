# Backlog

Work that is real but not in flight. Newest thinking at the top of each section;
items move out of here into a branch or an issue when they start.

Design documents live in [`exochain/`](exochain/) and [`sync/`](sync/); this file
is only the list.

---

## Blocked on someone else

| Item | Blocked by |
|---|---|
| Emit and verify receipts (R4/R5) — canonical `exo.avc.action.v1` signing | [exochain#812](https://github.com/exochain/exochain/issues/812) — `exochain-core` 0.2.3 does not build from crates.io |
| Swap the DID derivation to `did_from_public_key` | Same. The known-answer test guards the swap; DIDs must not change |
| Sanctioned emit path for a native-Rust subject | [exochained-toolkit#2](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/2) |
| `subject_kind` for a desktop install | [exochained-toolkit#3](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/3) |
| Endpoint → counterparty DID mapping, which turns evidence into enforcement | [exochained-toolkit#4](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/4) |
| Conformance suite and CI gate | toolkit phases P2–P3 are unbuilt |

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
- **Remote model endpoints** — no request has reached a live vLLM or remote
  Ollama. SSE reassembly is tested against captured frames, not a server.

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

Not started. Design in [`sync/DESIGN.md`](sync/DESIGN.md).

1. Extract the canonical transcript form into a shared crate — the service must
   recompute hashes with byte-identical code, not a reimplementation.
2. Service skeleton: axum, Entra JWT validation, device registration by DID.
3. Sync protocol: change feed, per-device cursors, tombstones, chain re-base.
4. Sharing: owner and explicit shares, Graph-resolved group membership.
5. Client: sign-in, background sync, "Remove from this device" versus "Delete
   from archive".
6. Receipts for mirroring and for sharing.
7. Retention policy support — never forgets by default.

## Build and release

- **CUDA CI build is unverified.** It builds locally; the workflow's CUDA job has
  never completed, having failed on Visual Studio integration and then on the
  GitHub Actions outage.
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
