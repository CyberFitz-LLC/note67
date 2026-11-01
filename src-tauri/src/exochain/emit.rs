//! Minting a meeting receipt.
//!
//! The app builds an action describing what it did, signs it with this
//! installation's key, and asks a node to validate it against the credential
//! and mint a receipt. The node decides; the app only ever reports what it was
//! told.
//!
//! Nothing here fabricates. An unreachable node yields `Pending` and a denial
//! yields `Denied` — never a receipt hash the app invented, and never a claim
//! that something was attested when it was not. That distinction is the entire
//! product.

use ed25519_dalek::{Signer, SigningKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use exochain_avc::{
    AutonomousVolitionCredential, AutonomyLevel, AvcActionRequest, AvcDecision, AvcReasonCode,
    DataClass,
};
use exochain_authority::permission::Permission;
use exochain_core::{Hash256, PublicKey, Signature, Timestamp};

use super::credential::MEETING_ATTEST_TOOL;

/// Identifies the action's shape. Frozen alongside the tool name: a receipt
/// anchors to the id derived here, so changing this would orphan every receipt
/// already minted.
pub const ACTION_DOMAIN: &str = "note67.action.v1|meeting-attest|";

/// The action id for a note.
///
/// Deterministic, so a retry after a dropped connection collapses onto the same
/// action rather than minting a second receipt for one meeting. That matters
/// more than it looks: receipts are the record, and two receipts for one
/// meeting is a record that disagrees with itself about how many there were.
pub fn action_id(note_id: &str) -> Hash256 {
    let mut hasher = Sha256::new();
    hasher.update(ACTION_DOMAIN.as_bytes());
    hasher.update(note_id.as_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    Hash256::from_bytes(digest)
}

/// The action name exochain v0.3.1 will accept for this credential.
///
/// From v0.3.1 the node checks `action_name` against the credential itself:
/// the part before `@` must equal one of `delegated_intent.allowed_objectives`
/// or extend one with `:`, must not fall under a prohibited objective, and the
/// part after `@` must be one of `authority_scope.jurisdictions` — required
/// whenever the credential lists any. The bare tool name the app used to send
/// is denied as `ObjectiveDenied` + `JurisdictionDenied`.
///
/// Both halves come from the credential rather than being hardcoded here, so a
/// re-minted credential with a different objective or jurisdiction keeps
/// working. When an allowed objective already covers the tool name, the tool
/// name is the objective; otherwise the tool is appended to the first usable
/// objective as `<objective>:note67.meeting.attest`.
///
/// Refuses, rather than guessing, when the credential grants no usable
/// objective: sending would only get a signed Deny receipt written to the
/// chain for a request that could never be allowed.
pub fn action_name(credential: &AutonomousVolitionCredential) -> Result<String, String> {
    let intent = &credential.delegated_intent;
    let prohibited = |claimed: &str| {
        intent
            .prohibited_objectives
            .iter()
            .any(|p| objective_covers(p, claimed))
    };
    let usable = intent
        .allowed_objectives
        .iter()
        .filter(|o| !o.is_empty() && !o.contains('@'));

    let mut objective = None;
    for allowed in usable.clone() {
        if objective_covers(allowed, MEETING_ATTEST_TOOL) && !prohibited(MEETING_ATTEST_TOOL) {
            objective = Some(MEETING_ATTEST_TOOL.to_string());
            break;
        }
    }
    if objective.is_none() {
        objective = usable
            .map(|allowed| format!("{allowed}:{MEETING_ATTEST_TOOL}"))
            .find(|claimed| !prohibited(claimed));
    }
    let objective = objective.ok_or_else(|| {
        format!(
            "This credential allows no objective that covers {MEETING_ATTEST_TOOL} \
             (allowed: {:?}, prohibited: {:?}). ExoChain 0.3.1 refuses every action \
             outside a credential's allowed objectives, so it needs re-minting with one.",
            intent.allowed_objectives, intent.prohibited_objectives
        )
    })?;

    Ok(match credential.authority_scope.jurisdictions.first() {
        Some(jurisdiction) => format!("{objective}@{jurisdiction}"),
        None => objective,
    })
}

/// The node's rule: `claimed` is `objective` or starts with `objective:`.
fn objective_covers(objective: &str, claimed: &str) -> bool {
    claimed == objective
        || claimed
            .strip_prefix(objective)
            .is_some_and(|rest| rest.starts_with(':'))
}

/// Refuse up front what v0.3.1 would refuse for the credential's autonomy.
///
/// The app asks for `Write` and never carries a human approval. v0.3.1 denies
/// `Write` below `Draft` (`AutonomyDenied`) and requires a signed approval at
/// `ExecuteWithHumanApproval` (`HumanApprovalMissing`), so either is a request
/// that can only end in a Deny receipt.
pub fn check_autonomy(credential: &AutonomousVolitionCredential) -> Result<(), String> {
    match credential.delegated_intent.autonomy_level {
        AutonomyLevel::ExecuteWithinBounds | AutonomyLevel::DelegateWithinBounds => Ok(()),
        AutonomyLevel::ExecuteWithHumanApproval => Err(
            "This credential's autonomy level is ExecuteWithHumanApproval, so ExoChain 0.3.1 \
             requires a signed human approval for every Write and this app does not carry one. \
             It needs re-minting at ExecuteWithinBounds."
                .to_string(),
        ),
        level => Err(format!(
            "This credential's autonomy level is {level:?}, which ExoChain 0.3.1 does not allow \
             to Write. It needs re-minting at ExecuteWithinBounds."
        )),
    }
}

/// What the app asks the node to attest.
///
/// `Write` and `note67.meeting.attest` must match the credential's authority
/// scope exactly — the node compares by equality, so a near-miss is a denial
/// rather than a warning. `action_name` comes from [`action_name`].
pub fn build_action(
    note_id: &str,
    actor_did: &exochain_core::Did,
    action_name: String,
) -> AvcActionRequest {
    AvcActionRequest {
        action_id: action_id(note_id),
        actor_did: actor_did.clone(),
        requested_permission: Permission::Write,
        tool: Some(MEETING_ATTEST_TOOL.to_string()),
        target_did: None,
        // A transcript is identifiable speech by named participants.
        data_class: Some(DataClass::PersonalData),
        estimated_budget_minor_units: None,
        estimated_risk_bp: None,
        human_approval: None,
        requires_human_approval: false,
        action_name: Some(action_name),
    }
}

/// The bytes this installation signs.
///
/// Produced by the ExoChain crate rather than assembled here. The node
/// recomputes them the same way, and a reimplementation that drifted by one
/// byte would fail every signature with nothing to say why.
pub fn signature_payload(
    credential: &AutonomousVolitionCredential,
    action: &AvcActionRequest,
    now: &Timestamp,
) -> Result<Vec<u8>, String> {
    exochain_avc::avc_action_signature_payload(credential, action, now)
        .map_err(|e| format!("could not build the signing payload: {e}"))
}

/// The request body the node's emit endpoint accepts.
#[derive(Debug, Serialize)]
pub struct EmitRequest {
    pub validation: ValidationRequest,
    pub subject_signature: Signature,
    /// Supplied so a subject whose key the node has never registered can still
    /// be verified: the node derives a DID from it and checks it matches the
    /// actor. Without this a correctly registered credential is still refused
    /// with "subject public key is unresolved" — exochain#687.
    pub subject_public_key: Option<PublicKey>,
}

#[derive(Debug, Serialize)]
pub struct ValidationRequest {
    pub credential: AutonomousVolitionCredential,
    pub action: Option<AvcActionRequest>,
    pub now: Timestamp,
}

/// The node's answer.
///
/// Only the fields the app acts on; the rest of the response is ignored. Since
/// v0.3.1 a refusal can come back as `200` with a signed receipt whose
/// decision is `Deny`, so `receipt_hash` alone proves nothing — the decision
/// has to be read.
#[derive(Debug, Deserialize)]
pub struct EmitResponse {
    pub receipt_hash: String,
    #[serde(default)]
    pub exochain_finality_hash: Option<String>,
    #[serde(default)]
    pub receipt: Option<DecisionOnly>,
    #[serde(default)]
    pub validation: Option<ValidationOutcome>,
}

#[derive(Debug, Deserialize)]
pub struct DecisionOnly {
    pub decision: AvcDecision,
}

#[derive(Debug, Deserialize)]
pub struct ValidationOutcome {
    pub decision: AvcDecision,
    #[serde(default)]
    pub reason_codes: Vec<AvcReasonCode>,
}

/// What happened, in terms the app can show without overstating it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "camelCase")]
pub enum Attestation {
    /// The node minted a receipt.
    #[serde(rename_all = "camelCase")]
    Attested { receipt_hash: String },
    /// The node could not be reached. The transcript and its chain are
    /// unaffected; this can be retried, and the action id makes retrying safe.
    #[serde(rename_all = "camelCase")]
    Pending { reason: String },
    /// The node refused. Retrying will not help until something changes —
    /// usually the credential's scope, or its expiry.
    #[serde(rename_all = "camelCase")]
    Denied { reason: String },
}

/// Assemble and sign the request.
pub fn build_request(
    credential: &AutonomousVolitionCredential,
    key: &SigningKey,
    note_id: &str,
    now: Timestamp,
) -> Result<EmitRequest, String> {
    check_autonomy(credential)?;
    let action = build_action(note_id, &credential.subject_did, action_name(credential)?);
    let payload = signature_payload(credential, &action, &now)?;
    let signature = key.sign(&payload);

    Ok(EmitRequest {
        validation: ValidationRequest {
            credential: credential.clone(),
            action: Some(action),
            now,
        },
        subject_signature: Signature::Ed25519(signature.to_bytes()),
        subject_public_key: Some(PublicKey::from_bytes(key.verifying_key().to_bytes())),
    })
}

pub fn now() -> Timestamp {
    Timestamp {
        physical_ms: chrono::Utc::now().timestamp_millis().max(0) as u64,
        logical: 0,
    }
}

/// Send it.
///
/// Distinguishes "could not ask" from "was refused", because they need
/// different responses from the user and the difference is invisible in a
/// single error string.
pub async fn emit(
    client: &reqwest::Client,
    node_url: &str,
    token: Option<&str>,
    request: &EmitRequest,
) -> Attestation {
    let url = format!("{}/api/v1/avc/receipts/emit", node_url.trim_end_matches('/'));
    let mut sending = client.post(&url).json(request);
    if let Some(token) = token.filter(|t| !t.trim().is_empty()) {
        sending = sending.bearer_auth(token.trim());
    }

    let response = match sending.send().await {
        Ok(r) => r,
        // Offline, DNS, TLS, timeout. The meeting still happened and the
        // transcript is still hashed; only the attestation is missing.
        Err(e) => {
            return Attestation::Pending {
                reason: format!("the node could not be reached: {e}"),
            };
        }
    };

    let status = response.status();
    let body = response.text().await;
    match body {
        Ok(body) => classify(status.as_u16(), &body),
        Err(e) if status.is_success() => Attestation::Pending {
            reason: format!("the node's answer could not be read: {e}"),
        },
        Err(_) => classify(status.as_u16(), ""),
    }
}

/// Turn the node's answer into an outcome, without overstating it.
///
/// Only a 2xx whose receipt AND validation both say `Allow` is attested. A
/// `Deny` — which v0.3.1 returns as `200` with a signed denial receipt — is a
/// denial, and the receipt hash is passed on in the reason rather than
/// recorded as if it attested the meeting. A 2xx that does not say what was
/// decided is not a receipt the app can record either; it stays Pending.
pub fn classify(status: u16, body: &str) -> Attestation {
    if (200..300).contains(&status) {
        let parsed = match serde_json::from_str::<EmitResponse>(body) {
            Ok(parsed) => parsed,
            // A success the app cannot read is not a receipt it can record.
            Err(e) => {
                return Attestation::Pending {
                    reason: format!("the node's answer could not be read: {e}"),
                };
            }
        };
        let receipt = parsed.receipt.as_ref().map(|r| r.decision);
        let validation = parsed.validation.as_ref().map(|v| v.decision);
        let refused = match (&receipt, &validation) {
            (_, Some(v)) if *v != AvcDecision::Allow => Some(*v),
            (Some(r), _) if *r != AvcDecision::Allow => Some(*r),
            _ => None,
        };
        if let Some(decision) = refused {
            let reasons = parsed
                .validation
                .map(|v| v.reason_codes)
                .unwrap_or_default();
            return Attestation::Denied {
                reason: format!(
                    "the node refused ({decision:?}: {reasons:?}) and recorded signed \
                     denial receipt {}",
                    parsed.receipt_hash
                ),
            };
        }
        return match receipt {
            Some(_) => Attestation::Attested {
                receipt_hash: parsed.receipt_hash,
            },
            None => Attestation::Pending {
                reason: format!(
                    "the node answered {status} but did not say whether it allowed the action; \
                     receipt {} is not recorded as an attestation",
                    parsed.receipt_hash
                ),
            },
        };
    }

    // 5xx is the node having a bad day; that is worth retrying. 503 in
    // particular is what v0.3.1 answers when it needs an external timestamp
    // authority it does not have. 4xx is the node saying no, and retrying an
    // unchanged request would just ask again.
    if (500..600).contains(&status) {
        Attestation::Pending {
            reason: format!("the node returned {status}: {body}"),
        }
    } else {
        Attestation::Denied {
            reason: format!("{status}: {body}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CREDENTIAL: &str = include_str!("../../tests/fixtures/credential.json");

    fn credential() -> AutonomousVolitionCredential {
        serde_json::from_str(CREDENTIAL).expect("the fixture is a real credential")
    }

    /// The fixture as it would have to be re-minted for exochain v0.3.1: an
    /// objective covering the tool, and an autonomy level that may Write
    /// without a human approval. Its issuer signature no longer matches, which
    /// none of the tests using it depend on.
    fn current() -> AutonomousVolitionCredential {
        let mut c = credential();
        c.delegated_intent.allowed_objectives = vec![MEETING_ATTEST_TOOL.to_string()];
        c.delegated_intent.autonomy_level = AutonomyLevel::ExecuteWithinBounds;
        c
    }

    fn action_for(c: &AutonomousVolitionCredential, note: &str) -> AvcActionRequest {
        build_action(note, &c.subject_did, action_name(c).unwrap())
    }

    fn key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    fn stamp() -> Timestamp {
        Timestamp {
            physical_ms: 1_786_600_000_000,
            logical: 0,
        }
    }

    #[test]
    fn a_real_credential_parses_into_the_nodes_type() {
        // The registry mints against the ceremony commit and the node runs a
        // newer crate. If those ever disagree about the credential, nothing
        // downstream works — and the failure would look like a bad signature.
        assert_eq!(
            credential().subject_did.as_str(),
            "did:exo:8MejPR8VCNWD3WYchtr2EAMSU3qz94FEdC8SwknGPjrE"
        );
    }

    #[test]
    fn the_action_id_is_the_same_every_time_for_a_note() {
        // What makes a retry safe. Without it, a dropped connection would mint
        // a second receipt for one meeting, and the record would disagree with
        // itself about how many there were.
        assert_eq!(action_id("note-1"), action_id("note-1"));
    }

    #[test]
    fn the_action_id_is_a_plain_sha256_of_domain_and_note() {
        // scripts/fetch-receipt.ps1 recomputes this in PowerShell to prove a
        // receipt is about a given meeting. Two implementations of one hash
        // will drift unless the value is pinned somewhere both can be checked
        // against — this is that somewhere.
        //
        //   SHA-256("note67.action.v1|meeting-attest|" + "test-note")
        let mut hasher = Sha256::new();
        hasher.update(b"note67.action.v1|meeting-attest|test-note");
        let expected: [u8; 32] = hasher.finalize().into();
        assert_eq!(action_id("test-note").as_bytes(), &expected);
        assert_eq!(
            hex::encode(action_id("test-note").as_bytes()),
            "f572fe306a5452c4dd9fdfb2b889c8340f31d4826e74f01cddeeb9d991eddca7"
        );
    }

    #[test]
    fn different_notes_get_different_action_ids() {
        assert_ne!(action_id("note-1"), action_id("note-2"));
    }

    #[test]
    fn the_action_asks_for_exactly_what_the_credential_grants() {
        // The node compares by equality, so a near-miss is a denial rather
        // than a warning.
        let c = current();
        let action = action_for(&c, "note-1");
        assert_eq!(action.requested_permission, Permission::Write);
        assert_eq!(action.tool.as_deref(), Some(MEETING_ATTEST_TOOL));
        assert_eq!(action.data_class, Some(DataClass::PersonalData));
        assert!(c.authority_scope.tools.iter().any(|t| t == MEETING_ATTEST_TOOL));
        assert!(c.authority_scope.permissions.contains(&Permission::Write));
    }

    #[test]
    fn the_action_names_this_installation_as_the_actor() {
        let c = current();
        assert_eq!(action_for(&c, "n").actor_did, c.subject_did);
    }

    #[test]
    fn a_signing_payload_is_produced() {
        let c = current();
        let action = action_for(&c, "note-1");
        let payload = signature_payload(&c, &action, &stamp()).unwrap();
        assert!(!payload.is_empty());
    }

    #[test]
    fn the_payload_covers_the_action() {
        // It must, or a signature would be valid for any action at all.
        let c = current();
        let a = signature_payload(&c, &action_for(&c, "note-1"), &stamp()).unwrap();
        let b = signature_payload(&c, &action_for(&c, "note-2"), &stamp()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn the_payload_covers_the_time() {
        let c = current();
        let action = action_for(&c, "note-1");
        let later = Timestamp {
            physical_ms: stamp().physical_ms + 60_000,
            logical: 0,
        };
        assert_ne!(
            signature_payload(&c, &action, &stamp()).unwrap(),
            signature_payload(&c, &action, &later).unwrap()
        );
    }

    #[test]
    fn the_request_carries_the_public_key() {
        // Without it, a correctly registered credential is still refused with
        // "subject public key is unresolved" — exochain#687.
        let request = build_request(&current(), &key(), "note-1", stamp()).unwrap();
        assert_eq!(
            request.subject_public_key,
            Some(PublicKey::from_bytes(key().verifying_key().to_bytes()))
        );
    }

    #[test]
    fn the_signature_verifies_against_the_key_that_made_it() {
        use ed25519_dalek::Verifier;
        let c = current();
        let request = build_request(&c, &key(), "note-1", stamp()).unwrap();
        let payload = signature_payload(&c, request.validation.action.as_ref().unwrap(), &stamp())
            .unwrap();

        let Signature::Ed25519(bytes) = request.subject_signature else {
            panic!("expected an Ed25519 signature");
        };
        key()
            .verifying_key()
            .verify(&payload, &ed25519_dalek::Signature::from_bytes(&bytes))
            .expect("the signature should verify over the payload the node checks");
    }

    #[test]
    fn the_request_serializes_under_the_names_the_node_reads() {
        let request = build_request(&current(), &key(), "note-1", stamp()).unwrap();
        let v = serde_json::to_value(&request).unwrap();
        assert!(v["validation"]["credential"].is_object());
        assert!(v["validation"]["action"]["action_id"].is_string() || v["validation"]["action"]["action_id"].is_array());
        assert!(v["subject_signature"]["Ed25519"].is_array());
        assert!(v["subject_public_key"].is_array() || v["subject_public_key"].is_string());
    }

    #[test]
    fn an_unreachable_node_is_pending_not_attested() {
        // The property the whole feature rests on: never a receipt the app
        // invented, and never a claim that something was attested when the
        // node was never asked.
        let outcome = Attestation::Pending {
            reason: "offline".into(),
        };
        let v = serde_json::to_value(&outcome).unwrap();
        assert_eq!(v["status"], "pending");
        assert!(v.get("receiptHash").is_none());
    }

    #[test]
    fn outcomes_are_distinguishable_on_the_wire() {
        // Pending and Denied need different responses from the user — retry
        // versus fix the credential — and the difference is invisible in a
        // single error string.
        let attested = serde_json::to_value(Attestation::Attested {
            receipt_hash: "abc".into(),
        })
        .unwrap();
        assert_eq!(attested["status"], "attested");
        assert_eq!(attested["receiptHash"], "abc");

        let denied = serde_json::to_value(Attestation::Denied {
            reason: "403: out of scope".into(),
        })
        .unwrap();
        assert_eq!(denied["status"], "denied");
    }

    // ---- exochain v0.3.1: action names -----------------------------------

    /// Runs the crate's own validator with nothing registered and returns the
    /// reason codes. Issuer and signature reasons are always present that way;
    /// the tests look only at the codes v0.3.1 added for action bounds, which
    /// depend on the credential and the action alone.
    fn bound_reasons(c: &AutonomousVolitionCredential, action: AvcActionRequest) -> Vec<AvcReasonCode> {
        let request = exochain_avc::AvcValidationRequest {
            credential: c.clone(),
            action: Some(action),
            now: stamp(),
        };
        let registry = exochain_avc::InMemoryAvcRegistry::new();
        exochain_avc::validate_avc(&request, &registry)
            .expect("validation runs")
            .reason_codes
            .into_iter()
            .filter(|r| {
                matches!(
                    r,
                    AvcReasonCode::ObjectiveDenied
                        | AvcReasonCode::JurisdictionDenied
                        | AvcReasonCode::AutonomyDenied
                        | AvcReasonCode::HumanApprovalMissing
                )
            })
            .collect()
    }

    #[test]
    fn the_action_name_carries_the_objective_and_jurisdiction() {
        assert_eq!(action_name(&current()).unwrap(), "note67.meeting.attest@US");
    }

    #[test]
    fn the_crate_accepts_the_name_the_app_builds() {
        // The point of deriving the name: v0.3.1's own validator finds nothing
        // wrong with the objective, the jurisdiction or the autonomy.
        let c = current();
        assert_eq!(bound_reasons(&c, action_for(&c, "note-1")), vec![]);
    }

    #[test]
    fn the_bare_tool_name_the_app_used_to_send_is_now_denied() {
        let c = current();
        let mut action = action_for(&c, "note-1");
        action.action_name = Some(MEETING_ATTEST_TOOL.to_string());
        assert_eq!(
            bound_reasons(&c, action),
            vec![AvcReasonCode::JurisdictionDenied]
        );
    }

    #[test]
    fn a_broader_objective_is_extended_with_the_tool() {
        let mut c = current();
        c.delegated_intent.allowed_objectives = vec!["meeting-records".into()];
        c.authority_scope.jurisdictions = vec!["CA".into(), "US".into()];
        let name = action_name(&c).unwrap();
        assert_eq!(name, "meeting-records:note67.meeting.attest@CA");
        assert_eq!(bound_reasons(&c, action_for(&c, "n")), vec![]);
    }

    #[test]
    fn an_objective_that_is_a_prefix_of_the_tool_covers_it() {
        let mut c = current();
        c.delegated_intent.allowed_objectives = vec!["note67.meeting.attest".into(), "other".into()];
        assert_eq!(action_name(&c).unwrap(), "note67.meeting.attest@US");
    }

    #[test]
    fn no_jurisdictions_means_no_suffix() {
        let mut c = current();
        c.authority_scope.jurisdictions.clear();
        assert_eq!(action_name(&c).unwrap(), MEETING_ATTEST_TOOL);
        assert_eq!(bound_reasons(&c, action_for(&c, "n")), vec![]);
    }

    #[test]
    fn a_prohibited_objective_is_skipped() {
        let mut c = current();
        c.delegated_intent.allowed_objectives = vec!["blocked".into(), "meeting-records".into()];
        c.delegated_intent.prohibited_objectives = vec!["blocked".into()];
        assert_eq!(
            action_name(&c).unwrap(),
            "meeting-records:note67.meeting.attest@US"
        );
    }

    #[test]
    fn a_credential_with_no_objectives_is_refused_before_anything_is_sent() {
        // The App Registry credential this app was enrolled with grants no
        // objectives, which v0.3.1 denies for every action name.
        let c = credential();
        assert!(c.delegated_intent.allowed_objectives.is_empty());
        let err = action_name(&c).unwrap_err();
        assert!(err.contains("re-minting"), "{err}");
        let mut action = action_for(&current(), "n");
        action.action_name = Some(MEETING_ATTEST_TOOL.to_string());
        assert!(bound_reasons(&c, action).contains(&AvcReasonCode::ObjectiveDenied));
    }

    #[test]
    fn the_enrolled_credentials_autonomy_cannot_write_without_approval() {
        let c = credential();
        assert_eq!(
            c.delegated_intent.autonomy_level,
            AutonomyLevel::ExecuteWithHumanApproval
        );
        assert!(check_autonomy(&c).is_err());
        assert!(build_request(&c, &key(), "n", stamp()).is_err());
        // Agrees with the crate: the same credential, given a name it accepts,
        // still fails on the missing approval.
        let mut named = c.clone();
        named.delegated_intent.allowed_objectives = vec![MEETING_ATTEST_TOOL.to_string()];
        let action = action_for(&named, "n");
        assert_eq!(
            bound_reasons(&named, action),
            vec![AvcReasonCode::HumanApprovalMissing]
        );
    }

    #[test]
    fn autonomy_below_draft_is_refused() {
        let mut c = current();
        c.delegated_intent.autonomy_level = AutonomyLevel::Recommend;
        assert!(check_autonomy(&c).is_err());
        assert!(bound_reasons(&c, action_for(&c, "n")).contains(&AvcReasonCode::AutonomyDenied));
    }

    // ---- exochain v0.3.1: reading the answer ------------------------------

    fn answer(receipt: &str, validation: &str, reasons: &str) -> String {
        format!(
            r#"{{"receipt_hash":"abc","exochain_finality_hash":"f","receipt":{{"decision":"{receipt}","other":1}},"validation":{{"decision":"{validation}","reason_codes":{reasons}}}}}"#
        )
    }

    #[test]
    fn an_allow_is_attested() {
        assert_eq!(
            classify(200, &answer("Allow", "Allow", "[]")),
            Attestation::Attested {
                receipt_hash: "abc".into()
            }
        );
    }

    #[test]
    fn a_200_deny_is_a_denial_not_a_receipt() {
        // v0.3.1 stores a signed Deny receipt and answers 200. Recording it as
        // an attestation would claim the meeting was attested when the node
        // said no.
        let outcome = classify(
            200,
            &answer("Deny", "Deny", r#"["ObjectiveDenied","JurisdictionDenied"]"#),
        );
        let Attestation::Denied { reason } = outcome else {
            panic!("expected Denied, got {outcome:?}");
        };
        assert!(reason.contains("ObjectiveDenied"), "{reason}");
        assert!(reason.contains("abc"), "{reason}");
    }

    #[test]
    fn a_deny_in_either_half_is_a_denial() {
        assert!(matches!(
            classify(200, &answer("Allow", "Deny", "[]")),
            Attestation::Denied { .. }
        ));
        assert!(matches!(
            classify(200, &answer("Deny", "Allow", "[]")),
            Attestation::Denied { .. }
        ));
    }

    #[test]
    fn a_200_that_does_not_say_what_was_decided_is_pending() {
        assert!(matches!(
            classify(200, r#"{"receipt_hash":"abc"}"#),
            Attestation::Pending { .. }
        ));
    }

    #[test]
    fn an_unreadable_200_is_pending() {
        assert!(matches!(classify(200, "not json"), Attestation::Pending { .. }));
    }

    #[test]
    fn a_503_is_pending_not_attested() {
        // What v0.3.1 answers when it needs an external timestamp authority it
        // does not have. Retryable, and certainly not a receipt.
        assert!(matches!(
            classify(503, "timestamp authority unavailable"),
            Attestation::Pending { .. }
        ));
    }

    #[test]
    fn a_403_is_denied() {
        assert!(matches!(
            classify(403, "AVC validation denied"),
            Attestation::Denied { .. }
        ));
    }
}
