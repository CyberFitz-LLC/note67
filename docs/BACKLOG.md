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
is answered by the PRD's own design.** Emission is an HTTP `POST` to a node's
`/api/v1/avc/receipts/emit`, not local signing, so "the sanctioned emit path for
a native-Rust subject" is a question about a client, and the client is ours to
write.

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
