# ExoChain build-out — decisions needed

Six choices block or shape the implementation. Three were filed as questions for
Bob and are still unanswered; three come from the scope change (Arizona
one-party consent, transcript import, external Postgres).

Each has a recommendation. None is irreversible, but D1 and D6 are expensive to
change later.

---

## D1 — How does a Rust subject sign and emit?

Filed as [exochained-toolkit#2](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/2).

R9 says to "ride the tested WASM adapter". That shape exists because every
partner so far is TypeScript, where WASM is the only route to exochain's
canonical encoder. Note67 is Rust and has a third option.

| Option | What it means | Cost |
|---|---|---|
| **A. Native `exo-avc`** | Depend on exochain's crates directly. `avc_llm_usage_action_request` already returns a complete `AvcActionRequest`. | R9 as written names WASM bindings, so conformance needs a contract amendment. |
| **B. WASM via `wasmtime`** | Run the sanctioned artifact literally. | A wasm runtime in the binary, plus an artifact to ship, pin and re-verify on every upgrade. |
| **C. Hand-roll the CBOR** | — | Explicitly forbidden by the onboarding docs. Not a real option. |

**Licensing is clear either way:** exochain is Apache-2.0, Note67 is
AGPL-3.0-or-later, and Apache-2.0 into AGPL is a permitted direction.

**Recommendation: A.** Going through WASM to re-enter Rust adds a runtime and an
artifact-versioning problem to a desktop app, to satisfy wording rather than
intent. The anti-drift guarantee R9 wants (that we ride tested code, not a stale
copy) is better served for a Rust subject by pinning the `exo-avc` version and
byte-parity testing against `avc_action_signature_payload`.

**If A, we deviate from R9 knowingly** and say so in the conformance report
rather than quietly failing it.

---

## D2 — One identity for the app, or one per install?

Filed as [exochained-toolkit#3](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/3).

`AvcSubjectKind` has no Device variant, so `Service { service_id }` is the only
defensible fit. The open part is what `service_id` contains.

| Option | Revocation granularity | Notes |
|---|---|---|
| **A. `note67`** | All installs at once | One credential, shared. **Requires sharing a private key**, which R3 forbids in spirit: the DID is derived from the signing key. |
| **B. `note67:<device-id>`** | Per machine | Each install generates its own keypair, derives its own DID, gets its own credential. |

**Recommendation: B.** You are about to run this on a second machine, so the
question is live rather than theoretical. Per-install also means a lost laptop
can be revoked without disturbing anything else.

Device id from a generated UUID stored beside the key, not a hostname —
hostnames change and collide.

---

## D3 — Enforce which model endpoints are allowed, or only record them?

Filed as [exochained-toolkit#4](https://github.com/apexvelocitycatalyst/exochained-toolkit/issues/4).

`counterparties` enforces at the node: if the list is non-empty, `target_did`
must be a member. But nothing maps `http://spark:8000/v1` to a DID.

| Option | Property |
|---|---|
| **A. Evidence only (v1)** | LYNK records `provider_endpoint` in every receipt. You can see afterwards where a transcript went. Nothing stops an unapproved endpoint. |
| **B. Static mapping** | Credential carries endpoint→DID pairs. Enforced — but authorizes *a string the user typed*, so DNS or a proxy defeats it. |
| **C. Endpoint presents its DID** | Genuinely authorizes the party that answers. Stock vLLM and Ollama cannot do this, so it needs a shim in front of every model server. |

**Recommendation: A for v1, C as the real answer later.** B costs implementation
effort for a guarantee that does not survive the threat it appears to address,
which is worse than being clear that v1 records rather than enforces.

---

## D4 — What do receipts attach to? *(changed by Arizona)*

Arizona is a one-party consent state and you are a party to these meetings, so
consent is not the constraint the PRD assumed. That frees the design to attest
what is actually valuable: the integrity and lineage of the record.

| Option | Receipts per | Gives you |
|---|---|---|
| **A. Transcript only** | One per finalized transcript | Existence + integrity at a point in time. The original PRD. |
| **B. Transcript versions** | One per version, chained | A tamper-evident history: the first transcript, and every later edit or re-transcription, each anchored and linked to its predecessor. |
| **C. Segment-level** | Per audio segment plus transcript | Finest provenance, most receipts, most complexity. |

**Recommendation: B.** "Track the transcript" is what you asked for, and a chain
is what makes a later edit visible rather than silent. It also handles
re-transcription honestly — running Improve transcript quality produces a new
version with its own receipt, rather than quietly invalidating the old one.

**Consent stays, minimised.** A single `consent_basis` field defaulting to
`OnePartyOperator`, because you will not always be in Arizona and a receipt that
records the basis costs nothing. The prompt from PRD §8.4 goes away as a
per-meeting interruption and becomes a setting.

**What a receipt still cannot say:** that the transcript is *accurate*. It
attests that this text came from that audio, unedited since, at that time.
Whisper's mistakes are attested faithfully as Whisper's mistakes.

---

## D5 — Imported transcripts claim less, and must say so

Importing Teams transcripts is straightforward. The receipt semantics are not,
and getting this wrong would be the most damaging error in the whole design.

For a Note67 recording we observe the whole pipeline: audio captured here,
transcript produced here, both hashed. For an imported file we observe **only
that a file with hash H was imported at time T**. We cannot attest it reflects a
real meeting, that it is complete, or that it was not edited before import.

So imported receipts carry `origin: imported`, plus the source tool and original
filename, and the UI must not show them as equivalent. Proposed wording:

> Imported from Microsoft Teams on 8 Aug 2026. Note67 attests only that this
> file was imported unchanged since; it did not produce it.

**Formats:** Teams exports `.vtt` and `.docx`. VTT is structured and trivial to
parse into timed segments; DOCX needs unpacking and yields poorer structure.
Recommend **VTT first**, plain text second, DOCX only if you need it.

---

## D6 — How does external Postgres relate to the local database?

| Option | Offline behaviour | Notes |
|---|---|---|
| **A. Replace SQLite** | **Recording stops when the database is unreachable** | Simplest data model, destroys the offline-first property that makes the app usable on a plane. |
| **B. SQLite primary, Postgres mirror** | Unchanged; sync when reachable | Local stays authoritative. Postgres is a durable, queryable archive. Mirrors how receipts already handle Pending. |
| **C. Postgres for receipts + transcripts only** | Partial | Splits one logical record across two stores; queries and consistency get awkward. |

**Recommendation: B.** It matches the shape already chosen for attestation — act
locally, reconcile when the network allows — and it keeps a database outage from
becoming a recording outage.

**Connection details** go in the settings store like the AI provider config, with
the password write-only from the UI (same treatment as the model API key).
Require TLS by default.

**This does not satisfy R8.** DAG-DB is ExoChain's governed store with its own
routes; an external Postgres of our own is not the same thing and does not move
us toward that requirement. R8 stays excluded.

---

## Sequencing once decided

1. Identity + keypair + DID derivation (needs D1, D2)
2. Transcript versioning + receipt chain (needs D4)
3. Emit + verify against a node (needs D1)
4. Import (needs D5)
5. Postgres mirror (needs D6)

1–2 are independent of any node being reachable, so they can land first and be
useful on their own.
