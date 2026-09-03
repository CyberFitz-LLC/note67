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

- ~~**No echo suppression.**~~ Done 2026-08-20. The filter is shared with the
  local path; what differs is the time frame. The two track clocks count what
  each socket was sent, and Windows loopback delivers nothing while no audio
  plays, so the system track's absolute offset drifts behind the microphone's
  and the two are not comparable. Utterances are therefore placed by *arrival*,
  extended backwards by their duration — the one thing both clocks measure
  reliably. **Not yet confirmed against a real meeting on speakers.**
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

1. ~~**The level meter only ever measured the microphone.**~~ Fixed 2026-08-26.
   `process_audio` in `recorder.rs` was the only writer of `audio_level`, and
   it is the *input* stream's callback, so the meter could never move for
   system audio. The recording bar now shows one meter per track, both fed
   from `audio::levels::LevelMeter` — which the loopback path had been writing
   to all along.
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

**Design written 2026-08-21: [`people/DESIGN.md`](people/DESIGN.md).** Phase 1
(named people, participants, filtering by person) is scoped and carries no
biometric claim; three open questions there need answering before it starts.
The notes below are what that design was built from.

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

## Structured notes, after seeing Teams Facilitator

Observed 2026-09-03 in a real Teams meeting and reported as "the best note
taking thing I've seen so far": screenshots captured automatically and
**organised by topic**, summaries of the thinking, an outline of what was
discussed.

Worth separating what is close from what is a different product, because they
look alike in a demo and are not alike to build.

**Topic segmentation is the keystone, and we are nearest to it.** Everything
else Facilitator does well is downstream of dividing a meeting into topics:
"screenshots by topic" and "a summary per topic" both fall out of it once the
divisions exist, and the transcript stops being a wall of text. The live brief
already reads a running meeting and says what is being discussed — it just
throws the structure away and keeps the prose. Asking the same pass for
boundaries as well as a summary is a small change to a prompt and a large
change to what the note looks like afterwards.

The hard part is not detecting a topic change. It is that the divisions have to
be **stable**: a boundary that moves every ninety seconds as the model
reconsiders makes a note that cannot be read while the meeting is still
running. Once a topic has closed it should stay closed, which means the pass
proposes new boundaries rather than re-segmenting the whole meeting each time —
the same accumulator shape the brief already uses, for the same reason.

**Organising what we already capture is then mostly presentation.** Screenshots
carry the meeting position they were pasted at, and transcript segments carry
times. Both can be placed under a topic by timestamp without any new capture,
new model call, or new storage. This is the cheapest large win in the whole
idea.

**Automatic screenshots are a different product.** Teams can do it because it
*is* the meeting client and already holds the shared-screen stream. Note67 sees
system audio, not video, so the equivalent is periodically capturing the
screen — which is a materially different privacy claim from recording a call,
belongs in `SCOPING.md` on its own row rather than under an existing one, and
on the machine that runs it competes with the video encode that has already
caused trouble once. Worth doing deliberately or not at all.

**What Facilitator has that we structurally cannot.** It knows the participant
list, who is speaking from the call's own metadata, and when someone joined or
left. We have two audio tracks and whatever a diarizer can infer. Any comparison
should be honest about that: the note can be as well organised, but attribution
will not be as good until [`people/DESIGN.md`](people/DESIGN.md) lands, and
even then it is inference rather than knowledge.

**Order, if this is picked up:** topics from the existing brief pass; then place
the screenshots and transcript already captured under them; then consider
capture. Nothing before the first of those is worth building, and the first is
worth building whether or not the rest ever is.

## Live meeting assistance

**Design written 2026-08-29: [`live-assist/DESIGN.md`](live-assist/DESIGN.md).**
Two panes beside a running meeting — a rolling brief and reactive suggestions
drawing on Hindsight recall — with the response options offered as buttons that
steer a second, focused pass. Four open questions there, of which the memory
bank and the receipt-per-session ruling both block a start.

## Retranscription with a remote recogniser — parked 2026-08-29

Built and never made to work end to end. It crashed the appliance twice, and
on the run that did complete, the client did not apply the result. The failures
were mostly memory on the appliance rather than the client, but the feature
never delivered a diarized transcript to a user.

**Parked rather than removed**: `retranscribe_remote` and its path still work
if the appliance has room, and the useful half — diarizing a finished recording
to get speakers a live recogniser cannot — is worth reviving another way.
[`live-assist/DESIGN.md`](live-assist/DESIGN.md) does not depend on it. The
likelier revival is local diarization over the existing transcript
(`sherpa-onnx` has Rust bindings and models measured in tens of megabytes), so
no appliance is involved at all.

## Note67 app

- **A capture buffer with no consumer used to grow for the length of the
  recording.** Reported 2026-09-02 from a two-hour meeting: audio went
  scratchy, video showed interference, and the machine had to be abandoned
  mid-call at about ninety minutes. Note67 was the cause, though not for the
  reason first suspected — nothing compresses during recording, and the
  playback mix is only rebuilt on stop.

  Only the transcription consumer drains `audio_buffer` and the system buffer.
  When nothing is transcribing — no model loaded, live transcription never
  started, or the streaming feed loop having stopped because its socket died —
  the capture callbacks carried on filling them: roughly 1.6 GB an hour between
  the two tracks, on a machine also carrying a video call.

  Both buffers are now bounded to about thirty seconds, oldest first. **The
  bound is not the whole fix**: the streaming feed loop still stops draining
  when a socket dies while the recording continues, so a dropped recogniser
  silently costs the rest of the meeting's live transcript. That is worth
  fixing on its own terms rather than relying on a memory bound to make it
  survivable.

- **Retranscribing a long meeting can take the appliance down with it.**
  2026-08-26: a diarizing retranscribe of a fifty-minute call rebooted the
  Spark. The cause is on that box — its containers run with no memory limit
  (`Memory=0`), so when NeMo diarization asked for headroom that a resident
  27B model had already taken, the kernel had no container to sacrifice and
  the machine went instead. Memory limits on the containers are the fix, and
  they are not ours to set.

  What we changed is the half of the load that was never needed: the
  microphone track asks for one speaker rather than diarizing a recording that
  has one person in it by construction. That roughly halves the heavy work.
  It reduces the risk; it does not remove it, and nothing in this app can —
  a client cannot see how much memory an appliance has left. Chunking long
  audio would bound it, at the cost of speaker identity across chunk
  boundaries, which is the whole point of diarizing.

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
- **Recordings can be left unfinalized, and used to be unreadable.** hound
  writes the WAV header with placeholder lengths and patches them on
  `finalize()`; if the app dies first the samples are all on disk and every
  decoder refuses the file. Nine such files turned up in one real library.
  `codec::recover_unfinalized_wav` now reads them, so the audio is not lost.

  The cause was app crashes, which John reports have largely stopped. Nothing
  prevents the state and nothing announces it — a recording that ends this way
  is invisible until something reads it — but with recovery in place the
  practical harm is gone, so this is **not** worth building crash detection
  for. If unfinalized files start appearing again in fresh recordings, that is
  the signal that something is crashing once more, and it is worth chasing
  then rather than now.
- **Deleting a note leaves its audio on disk, for ever.** The rows go — the
  FK cascades — and the files are never touched. Compaction now reports how
  many it found that nothing references, which on one real library was a large
  fraction of the directory. Shrinking them is not the same as removing them,
  and removing them is a deletion path that does not exist yet.
- **Recordings are never pruned.** Much less pressing since 2026-08-21:
  recordings are 16 kHz mono FLAC, about an eighth of what they took, and
  Settings → System compacts an existing library. What remains is that nothing
  ever deletes anything.
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

- ~~**CUDA CI build has never succeeded.**~~ Green since 2026-08-21. It was
  toolchain drift, not our code: CI was on CUDA 12.5 against a Visual Studio 18
  runner image, while the local script had already moved to CUDA 13. What the
  toolchain requires, and why each flag is there, is written down in
  [BUILD.md](BUILD.md) — read that before changing anything CUDA-shaped.
  Two things it left open, both listed there: locally built installers are
  probably GPU-specific, and the CUDA installer is 862 MB against Vulkan's 20.

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
