# Making Note67 ExoChained — scoping

Status: **proposal, not started.** Written 2026-08-05 against
`exochained-toolkit` @ `dcff78a`, whose contract is pinned to exochain
`7b70f649`.

## The bar

From `contract/exochained.contract.v1.yaml`: a tool is ExoChained iff it passes
every requirement in its deployment's `required_set`. Enforcement is hard
fail-closed and CI-gated. Absence of positive evidence scores **fail**, never
skip.

R1 root-issued credential · R2 signature-gated registration · R3 subject key
self-certifies to its DID · R4 canonical `exo.avc.action.v1` signing · R5
node-signed receipts · R6 fail-closed governance validation · R7 durability +
RFC-3161 · R8 governed state via DAG-DB routes · R9 the tested external-signing
WASM path.

## The tension to resolve first

Note67's product claim is "private, local, works on your device." ExoChain's is
"no reachable node, no action." R6 is explicit: *node-unreachable → the tool must
block*. Applied across the whole app, **a laptop on a plane cannot record a
meeting.**

That is not a defect in either system. It is what happens when a fail-closed
governance gate is put in front of an offline-first tool, and it has to be
decided deliberately rather than discovered late.

## Proposed scope: govern egress, leave capture local

| Action | Governed | Why |
|---|---|---|
| Record / transcribe locally | No | Purely on-device; offline must keep working |
| Summarize via a **remote** model endpoint | **Yes** | Transcript leaves the machine |
| Export / share a note | **Yes** | Provenance leaves the user's control |
| Change the configured model endpoint | **Yes** (receipt) | Rare, high-leverage, the moment policy would be subverted |

This became worth doing only recently. Before the AI-provider work
(`src-tauri/src/ai/provider.rs`), no transcript could leave the device at all.
Now it can, and the gate belongs exactly at that boundary.

The resulting claim is honest and narrow: *meeting content only goes where policy
allows, and every transit is signed.* The app stays fully functional offline.

### Excluded, deliberately

- **R8 (DAG-DB governed state).** Notes live in local SQLite. Routing note
  writes through `POST /api/v1/dag-db/*` would delete offline operation. Exclude
  from the `required_set`.

## Why the action model fits

Validation is exact-match against the credential
(`exo-avc/src/validation.rs`, per `docs/AVC-EMIT-CONTRACT.md`):

- `tool` must be an exact member of `authority_scope.tools`; an empty list denies
  everything.
- `data_class` must be an exact member of `authority_scope.data_classes`.
- **`counterparties`: if non-empty, `target_did` must be a member.**

That last rule is the control. Each approved model endpoint is a counterparty
DID. The Spark is in the list and works; an unapproved cloud endpoint is not and
is denied *at the node*, not by a local checkbox.

## Where the gates go

The provider abstraction added in `feat/input-device-selection` put every AI call
behind one function, so the enforcement point already exists:

| Seam | File | Action |
|---|---|---|
| All AI generation | `src-tauri/src/commands/ai.rs` → `AiState::client()` | Validate before returning a client when the provider is remote. Local loopback stays ungated. |
| Endpoint change | `src-tauri/src/commands/ai.rs` → `set_ai_provider_config` | Emit a receipt. |
| Export | `src-tauri/src/commands/export.rs` | Validate + receipt. |

All 13 generation call sites route through `AiState::client()`, so gating there
covers summaries, titles, action items, and the writing assistant without
touching any of them.

## Draft credential

`note67-avc-draft.json`, modelled on
`exochain-partner-onboarding/partners/omertasec/drafts/omerta-avc-draft.json`.
The four must-fill fields (`subject_did`, `created_at`, `expires_at`,
`intent_id`) keep the template's sentinels so it cannot be minted by accident.

`tools` and `allowed_objectives` are sorted — canonical signing makes ordering
load-bearing, not cosmetic.

## Effort

The app-side work is modest because the seam exists. The unbudgeted item is the
adapter.

| Phase | Work | Size |
|---|---|---|
| 1 | Identity (Ed25519 on-host, 0600) + canonical DID + register via `avc-app-registry` | ~1 day |
| 2 | **Rust emit adapter** with byte-parity tests against `avc_action_signature_payload` | unknown — see open questions |
| 3 | Gate + receipts at the three seams | ~2–3 days |
| 4 | Conformance suite + CI gate | blocked, see below |

### Blocked on the toolkit

`docker/` and `harness/live/` are both marked "Planned contents"; the hermetic
node, live checks, and the `exochain onboard` CLI are Phase 2–3 and unbuilt as of
the toolkit's last commit (2026-07-07). Note67 cannot be certified green until
those land, regardless of what is built here.

## Open questions (filed against `exochained-toolkit`)

1. **R9 for a native-Rust subject.** The emit flow is external signing: the WASM
   yields canonical CBOR payload bytes, the runtime signs them, the WASM
   assembles the request. That shape exists because every partner so far is
   TypeScript. Does a Rust subject run the sanctioned WASM under `wasmtime`, or
   use `exo-avc` natively — and does R9 still hold if it does?
2. **`subject_kind` for a desktop app.** `Service { service_id }` describes a
   service. Is a per-install desktop identity a Service?
3. **Endpoint → counterparty DID mapping.** How does the app learn that
   `http://spark:8000/v1` is `did:exo:…`? Via `policy_refs`, or in the
   credential?

Also unresolved, not yet filed:

4. Is `Confidential` in the `DataClass` enum at ref `7b70f649`? It appears in
   OmertaSec's Charter Protocol prose but was not confirmed in the enum.
5. **Fleet provisioning.** The onboarding scaffold mints for *a tool*. Many
   end-user installs is a model that does not exist yet. Fine for internal use;
   not fine for public distribution.

## Caveat worth stating before this is sold

The claims-vs-code audit (exochain #696–700) found the receipt path does not
support "provable chain / legally admissible." If the reason to ExoChain a
*meeting recorder* is courtroom-grade provenance, that gap is load-bearing and
should close first — otherwise the integration backs a claim the substrate does
not yet support.

## Recommendation

Govern egress, leave capture local. Exclude R8. Get a ruling on R9 before any
code. Hold implementation until toolkit P2–P3 land. Do not attempt public-fleet
credentialing on the current onboarding model.
