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

use exochain_avc::{AutonomousVolitionCredential, AvcActionRequest, DataClass};
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

/// What the app asks the node to attest.
///
/// `Write` and `note67.meeting.attest` must match the credential's authority
/// scope exactly — the node compares by equality, so a near-miss is a denial
/// rather than a warning.
pub fn build_action(note_id: &str, actor_did: &exochain_core::Did) -> AvcActionRequest {
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
        action_name: Some(MEETING_ATTEST_TOOL.to_string()),
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
#[derive(Debug, Deserialize)]
pub struct EmitResponse {
    pub receipt_hash: String,
    #[serde(default)]
    pub exochain_finality_hash: Option<String>,
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
    let action = build_action(note_id, &credential.subject_did);
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
    if status.is_success() {
        return match response.json::<EmitResponse>().await {
            Ok(body) => Attestation::Attested {
                receipt_hash: body.receipt_hash,
            },
            // A success the app cannot read is not a receipt it can record.
            Err(e) => Attestation::Pending {
                reason: format!("the node's answer could not be read: {e}"),
            },
        };
    }

    let body = response.text().await.unwrap_or_default();
    // 5xx is the node having a bad day; that is worth retrying. 4xx is the node
    // saying no, and retrying an unchanged request would just ask again.
    if status.is_server_error() {
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
        let c = credential();
        let action = build_action("note-1", &c.subject_did);
        assert_eq!(action.requested_permission, Permission::Write);
        assert_eq!(action.tool.as_deref(), Some(MEETING_ATTEST_TOOL));
        assert_eq!(action.data_class, Some(DataClass::PersonalData));
        assert!(c.authority_scope.tools.iter().any(|t| t == MEETING_ATTEST_TOOL));
        assert!(c.authority_scope.permissions.contains(&Permission::Write));
    }

    #[test]
    fn the_action_names_this_installation_as_the_actor() {
        let c = credential();
        assert_eq!(build_action("n", &c.subject_did).actor_did, c.subject_did);
    }

    #[test]
    fn a_signing_payload_is_produced() {
        let c = credential();
        let action = build_action("note-1", &c.subject_did);
        let payload = signature_payload(&c, &action, &stamp()).unwrap();
        assert!(!payload.is_empty());
    }

    #[test]
    fn the_payload_covers_the_action() {
        // It must, or a signature would be valid for any action at all.
        let c = credential();
        let a = signature_payload(&c, &build_action("note-1", &c.subject_did), &stamp()).unwrap();
        let b = signature_payload(&c, &build_action("note-2", &c.subject_did), &stamp()).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn the_payload_covers_the_time() {
        let c = credential();
        let action = build_action("note-1", &c.subject_did);
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
        let request = build_request(&credential(), &key(), "note-1", stamp()).unwrap();
        assert_eq!(
            request.subject_public_key,
            Some(PublicKey::from_bytes(key().verifying_key().to_bytes()))
        );
    }

    #[test]
    fn the_signature_verifies_against_the_key_that_made_it() {
        use ed25519_dalek::Verifier;
        let c = credential();
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
        let request = build_request(&credential(), &key(), "note-1", stamp()).unwrap();
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
}
