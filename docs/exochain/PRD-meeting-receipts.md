# PRD — Meeting Receipts (ExoChain) for Note67

| | |
|---|---|
| Status | Draft for review |
| Author | John Fitzpatrick |
| Date | 2026-08-05 |
| Depends on | exochain `86e9a029`, `exochained-toolkit` `dcff78a` |
| Blocked by | exochained-toolkit [#2](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/2), [#3](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/3), [#4](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/4); toolkit phases P2–P3 |
| Companion | [`SCOPING.md`](SCOPING.md), [`note67-avc-draft.json`](note67-avc-draft.json) |

## 1. Summary

Give every meeting Note67 records a **cryptographic receipt**: a node-signed,
RFC-3161-anchored attestation that a specific transcript existed, was produced by
a specific device holding a specific credential, and — where the AI assistant was
used — that its content went only to model endpoints the credential names.

The receipt carries **hashes, never content**. Meeting material stays on the
user's device.

## 2. Why now

Two things changed.

**The app grew a network boundary.** Until the AI-provider work, no transcript
could leave the machine. Note67 can now send transcripts to a remote Ollama or
any OpenAI-compatible endpoint. That is a genuine egress path with no record of
what went where.

**ExoChain grew the right primitive.** exochain `e5609e45` shipped the EXOCHAIN
LYNK Protocol LLM usage receipt, whose `custody_mode: ReceiptMinimized` proves a
model call happened using hashes alone. That is the shape a privacy-first app
needs, and it did not exist a month ago.

## 3. Goals

- **G1** Every finalized meeting can produce a receipt anchoring its transcript
  hash to a signed, externally-timestamped record.
- **G2** Every transcript sent to a remote model endpoint produces a LYNK usage
  receipt naming the provider, endpoint, and model.
- **G3** A receipt is verifiable by a third party who does not trust Note67 or
  the user, via `GET /api/v1/avc/receipts/{hash}`.
- **G4** Recording **never** depends on network or node availability.
- **G5** The user can always tell whether a meeting is attested, and if not, why.

## 4. Non-goals

- **N1** Proving *when a meeting occurred.* See §6.
- **N2** Proving who was in the room. Note67 has no participant identity.
- **N3** Gating recording or local transcription behind governance.
- **N4** Putting transcripts, audio, or summaries into ExoChain or DAG-DB.
- **N5** Public-fleet credential provisioning (see Risks).
- **N6** Legal admissibility as a claim. See §6 and R4 in Risks.
- **N7** Determining whether consent was legally sufficient in a jurisdiction.
  The app records an assertion and a jurisdiction; it does not evaluate them.

## 5. Users

**Meeting owner (primary).** Records client and internal calls. Wants to show
later that a transcript is the one produced at the time and hasn't been edited,
and wants confidence that meeting content didn't reach an unapproved AI endpoint.

**Compliance / admin.** Wants to set which model endpoints are permitted, revoke
a lost laptop, and answer "what left this device and where did it go" from
signed evidence rather than logs the user could edit.

**Third-party verifier.** A counterparty or auditor handed a transcript and a
receipt hash. Verifies against the node without needing anything from us.

## 6. What a receipt proves — and what it does not

This section is the product. Getting it wrong makes the feature a liability.

**A meeting receipt proves:**

- A transcript with hash `H` existed no later than the receipt's RFC-3161
  timestamp.
- It was attested by the subject holding credential `C`, on a device controlling
  the corresponding private key.
- That credential was issued by the AVC root and was valid, in scope, and
  unrevoked at attestation time.
- The transcript has not changed since: re-hash it and compare.

**It does not prove:**

- **When the meeting happened.** The timestamp is when the receipt was minted. A
  meeting recorded on a plane and attested that evening carries the evening's
  timestamp. The receipt bounds the transcript's existence *from above*, nothing
  more.
- **That the transcript is accurate.** Whisper output is attested as-is,
  hallucinations included.
- **Who spoke.** No participant identity exists.
- **That recording was consented to.** Where a consent basis is present (§8.4),
  the receipt proves only that *the operator asserted* that basis at that time.
  It is evidence of an assertion, never evidence of consent. A false assertion
  produces a valid receipt.

Every surface stating a receipt's meaning must use this language. "Verified" on
its own is not acceptable UI copy.

## 7. User experience

### 7.1 Enrollment (first run)

A new **Governance** step in the onboarding wizard, skippable. Skipping leaves
the app exactly as it is today — all receipt features simply absent. Enrolling
generates an Ed25519 keypair on-device (`0600`, never leaves the host), derives
the canonical DID, and registers against `avc-app-registry`.

**Recording is never blocked by enrollment state.**

### 7.2 During and after a meeting

Recording is unchanged. On finalization the app hashes the transcript, emits an
attestation, and stamps the note with one of:

| State | Meaning | UI |
|---|---|---|
| **Attested** | Receipt minted; hash shown, copyable | Badge + timestamp |
| **Pending** | Node unreachable; queued | Badge + "Attest now" |
| **Unattested** | Not enrolled, or user declined | Quiet marker |
| **Failed** | Node returned Deny | Badge + reason code |

**Pending is a first-class state, not an error.** Offline is the normal case for
a laptop, and the design must not shame it.

### 7.3 Attesting later

A note in **Pending** attests on next connectivity, automatically or on demand.
The resulting receipt carries the *attestation* time and the UI says so
explicitly: *"Transcript existed by 5 Aug 2026 19:42. Recorded earlier;
attestation time is not the recording time."*

### 7.4 The AI assistant

Summarizing to a **local** endpoint is unchanged and ungated.

Summarizing to a **remote** endpoint requires an in-scope credential. If the
endpoint isn't permitted, the app blocks the call and says which endpoint was
refused. If it is permitted, the call proceeds and mints a LYNK usage receipt.
Both outcomes are visible; neither is silent.

### 7.5 Export

Exports may carry a provenance block — subject DID, transcript hash, receipt
hash, verification URL — so a recipient can check it without trusting the sender.
Opt-in per export.

### 7.6 Admin

- Choose permitted model endpoints; changes propagate by credential.
- Revoke a device; its receipts stop minting and remote AI stops working, while
  local recording continues.
- Review conformance status per release in `avc-app-registry`.

## 8. Technical design

### 8.1 Subject and credential

Per [`note67-avc-draft.json`](note67-avc-draft.json). `AvcSubjectKind::Service {
service_id }` — the only defensible variant today, since no Device variant exists
(toolkit #3).

`data_classes` includes **`PersonalData`**: a transcript is identifiable speech by
named participants.

### 8.2 Two receipt kinds

| | Meeting attestation | LLM usage |
|---|---|---|
| Trigger | Transcript finalized | Remote model call |
| Protocol | Bespoke `exo.avc.action.v1` | **EXOCHAIN LYNK Protocol** |
| Tool | `note67.meeting.attest` | `exo.avc.lynk.llm_usage.evidence.v1` |
| Permission | `Write` | `Execute` |
| Route | `POST /api/v1/avc/receipts/emit` | `POST /api/v1/avc/llm-usage/receipts/emit` |
| Payload | Hashes + counts | `LlmUsageEvidence`, `custody_mode: ReceiptMinimized` |

Reusing LYNK rather than inventing a second usage receipt is deliberate: it
already carries `provider`, `provider_endpoint`, `model_id`, prompt and
completion hashes, and token/cost metrics — a one-to-one fit with what
`ProviderConfig` holds.

### 8.3 Meeting attestation contents

Hashes and counts only. No transcript text, no audio, no summary.

```
action_id      SHA-256("note67.action.v1|meeting-attest|" + note_id)
tool           note67.meeting.attest
data_class     PersonalData
transcript_hash    SHA-256 of the canonical transcript serialization
audio_hash         SHA-256 of the merged playback track (optional)
segment_count, duration_ms, model_id (whisper), app_version
```

`action_id` is deterministic per note, so retries collapse to one receipt rather
than minting duplicates.

**The canonical transcript serialization must be specified and frozen**, or a
re-hash years later won't match and the receipt becomes worthless. This is the
highest-risk detail in the document.

### 8.4 Consent attestation

Recording a conversation is legally significant, and in much of the US it is
regulated per-state rather than federally: some jurisdictions require only one
party to consent, others require all parties. Note67 captures nothing about this
today. A meeting receipt that is silent on consent is honest but of limited use
to the person who most needs it.

**What is attested is an assertion, not a fact.** The app cannot observe whether
consent was obtained. It can only record that the operator said so, when they
said it, and on what basis. Every field, string, and schema name below is chosen
to keep that distinction intact — a receipt that appears to certify consent
itself would be worse than no receipt at all.

#### Capture

A consent prompt appears when recording starts, before the first sample is
written. It is **not** pre-ticked and has no default. The user picks a basis:

| Basis | Meaning |
|---|---|
| `AllPartiesVerbal` | Everyone present was told and agreed, verbally |
| `AllPartiesWritten` | Written agreement exists (calendar notice, contract, policy) |
| `OnePartyOperator` | Operator is a party and relies on one-party consent |
| `NoticeDisplayed` | Platform recording notice was shown to all participants |
| `NotAsserted` | User declined to assert anything |

`NotAsserted` is a legitimate outcome and is recorded as such. **Declining does
not block recording** (goal G4) and does not produce a warning banner — the
choice is the user's and the receipt simply reflects it.

The prompt is dismissible-to-`NotAsserted` and can be disabled entirely in
settings for users who find it noise, in which case every meeting records
`NotAsserted`.

#### What lands in the receipt

Added to the meeting attestation payload (§8.3):

```
consent_basis          one of the values above
consent_asserted_at    local timestamp when the operator made the assertion
consent_jurisdiction   optional ISO-3166-2 (e.g. "US-WA"), user-set in settings
consent_policy_ref     optional; hash of an org consent policy from the credential
```

No participant names, no free text. A receipt must not become a place personal
data leaks by accident.

`consent_asserted_at` is captured at recording start and is therefore *earlier*
than the receipt's own timestamp. That ordering is the point: it shows the
assertion preceded the recording rather than being applied retroactively. It is
a local clock and is not itself attested — only the fact that this is what the
app recorded.

#### Org-level policy

Where an organization has a standing consent policy, its hash goes in the
credential's `consent_refs` and is echoed as `consent_policy_ref`. That binds a
meeting to the policy in force when it was recorded, so a later policy change
cannot rewrite what an old meeting claimed to follow.

#### Wording rules

Receipt-facing copy must attribute the assertion. Required form:

> Operator asserted all-party verbal consent, 6 Aug 2026 09:14 (US-WA).

Forbidden: "Consent verified", "Consented", a tick mark alone, or any string
implying the system observed consent. §6 governs; this is an instance of it.

### 8.5 Enforcement seams

| Seam | File | Behavior |
|---|---|---|
| AI generation | `commands/ai.rs` → `AiState::client()` | Validate when remote; local loopback ungated |
| Endpoint change | `commands/ai.rs` → `set_ai_provider_config` | Receipt |
| Export | `commands/export.rs` | Validate + receipt |
| Finalization | `commands/transcription.rs` | Meeting attestation |

All thirteen generation call sites already route through `AiState::client()`, so
one gate covers summaries, titles, action items, and the writing assistant.

### 8.6 Offline behavior — the deliberate divergence

The contract's fail-closed rule is *"Deny or node-unreachable → the tool must
block."* This design **blocks remote AI calls** on unreachable, and **does not
block recording**.

That is coherent because recording is not a governed action here: it touches no
counterparty and moves no data. Only egress and attestation are governed, and
both do fail closed — an unreachable node yields **Pending**, never a fabricated
receipt.

Stated plainly so nobody discovers it during conformance: **the `required_set`
excludes R8, and recording is outside the governed surface by design.**

## 9. Phasing

| Phase | Deliverable | Blocked by |
|---|---|---|
| **P0** | Identity + credential + enrollment UI | toolkit #2, #3 |
| **P1** | Meeting attestation, consent capture (§8.4), Pending/Attested states, frozen serialization | P0 |
| **P2** | LYNK usage receipts on remote AI | P1 |
| **P3** | Endpoint enforcement (counterparties) | toolkit #4 |
| **P4** | Verification UI + provenance export | P1 |
| **P5** | Conformance suite + CI gate | toolkit P2–P3 |

P1 is the smallest thing that delivers the headline feature. P3 is what turns
evidence into control — see Risks.

## 10. Risks

**R1 — Serialization drift kills old receipts.** If the canonical transcript
serialization changes, every prior receipt fails to verify. *Mitigation:* version
it, freeze v1, carry the version in the receipt, never re-serialize retroactively.

**R2 — Evidence is not enforcement.** LYNK's `provider_endpoint` records where
data went; it does not stop it going somewhere else. Without P3, "only approved
endpoints" is a claim the system does not make. *Mitigation:* do not market
endpoint control before P3 ships.

**R3 — Attestation time reads as meeting time.** Users will assume the timestamp
is when the meeting happened. *Mitigation:* §6 language everywhere; never show a
bare "verified" tick.

**R4 — Admissibility.** The claims-vs-code audit (exochain #696–700) found the
receipt path does not support "provable chain / legally admissible." *Mitigation:*
make no such claim until closed. This PRD deliberately claims only existence,
integrity, and authorship.

**R5 — A consent assertion reads as a consent guarantee.** The failure mode is a
user relying on a receipt in a dispute and discovering it attests only their own
claim. *Mitigation:* §8.4 wording rules, attribution in every string, and
`NotAsserted` treated as ordinary rather than deficient. Worth a legal read
before any external-facing copy.

**R6 — Fleet provisioning does not exist.** The onboarding scaffold mints for a
tool, not thousands of installs. *Mitigation:* internal use only until resolved.

**R7 — Node availability becomes a support burden.** A desktop app acquires a
server dependency for part of its function. *Mitigation:* Pending state means
outages degrade rather than break.

**R8 — Upstream merges break byte parity.** Every merge from ZapYap re-runs
conformance; R4/R5 byte-parity and R4 scope drift are the usual breakers.

## 11. Open questions

1. Sanctioned emit path for a native-Rust subject (toolkit #2). Blocks P0.
2. `subject_kind` and per-install identity (toolkit #3). Blocks P0.
3. Endpoint → counterparty DID mapping (toolkit #4). Blocks P3.
4. Does `consent_refs` carry a hash or a resolvable reference? §8.4 assumes a
   hash; confirm against the credential schema before P1.
5. Was LYNK designed with this consumer shape in mind? Worth asking Bob directly.
6. Should audio be hashed as well as the transcript? Anchors the source, but ties
   the receipt to a file users may delete.
7. Retention: what happens to a receipt when its note is deleted? The receipt
   outlives it by design — is that acceptable?

## 12. Success criteria

- A meeting recorded fully offline lands in **Pending** and attests cleanly on
  reconnection, with the UI stating what the timestamp means.
- A transcript sent to the Spark yields a LYNK receipt naming provider, endpoint,
  and model, containing no transcript content.
- A third party verifies a receipt hash against the node with nothing from us.
- Every user-facing string about a receipt survives review against §6.
- A meeting recorded with consent declined produces a valid receipt carrying
  `NotAsserted`, with no warning banner and no behavioural difference.
- No receipt-facing string states or implies that consent was verified.
- Recording works with the node down, the credential expired, and enrollment
  skipped.
