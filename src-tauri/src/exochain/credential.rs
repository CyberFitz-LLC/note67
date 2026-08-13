//! Holding the credential that says what this installation may do.
//!
//! The credential is minted elsewhere — the App Registry signs it with the
//! issuer key and registers it on the chain — and arrives here as JSON. This
//! module's job is to refuse the wrong one and to be honest about the state of
//! the right one.
//!
//! The shape modelled here is the **canonical AVC**, which is what the registry
//! actually emits: `authority_scope` and `delegated_intent` are objects,
//! timestamps are `{logical, physical_ms}`, and the signature is a tagged byte
//! array. An earlier version of this file modelled `exo_avc::SignedCredential`
//! — a flatter, registry-internal shape — and rejected every real credential
//! with "invalid type: map, expected a sequence".

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const CREDENTIAL_FILE: &str = "credential.json";

/// An ExoChain timestamp: wall clock in milliseconds, plus a logical counter
/// for ordering events that share one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AvcTime {
    pub physical_ms: i64,
    #[serde(default)]
    pub logical: i64,
}

/// What the credential permits.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AuthorityScope {
    #[serde(default)]
    pub permissions: Vec<String>,
    /// The tools this credential authorises, matched exactly by the node. An
    /// empty list authorises nothing at all.
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub data_classes: Vec<String>,
    #[serde(default)]
    pub counterparties: Vec<String>,
    #[serde(default)]
    pub jurisdictions: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DelegatedIntent {
    #[serde(default)]
    pub purpose: String,
    #[serde(default)]
    pub autonomy_level: String,
    #[serde(default)]
    pub delegation_allowed: bool,
}

/// A signed AVC, as the registry issues it.
///
/// Unknown fields are kept, because what is stored must stay byte-identical to
/// what was signed — and because this schema will grow.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credential {
    pub issuer_did: String,
    pub subject_did: String,
    pub created_at: AvcTime,
    pub expires_at: AvcTime,
    #[serde(default)]
    pub authority_scope: AuthorityScope,
    #[serde(default)]
    pub delegated_intent: DelegatedIntent,
    /// `{"Ed25519": [...]}`. Not interpreted here — the app does not verify the
    /// issuer's signature, the node does, and pretending otherwise would be a
    /// check that looks like security and is not.
    pub signature: serde_json::Value,
    #[serde(flatten)]
    pub rest: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("that is not a credential: {0}")]
    Malformed(String),
    #[error(
        "this credential is for a different installation (it names {subject}, this device is {ours})"
    )]
    WrongSubject { subject: String, ours: String },
    #[error(
        "this credential authorises no tools, so it permits nothing. Add {expected} to the \
         authority scope when minting it"
    )]
    NoTools { expected: &'static str },
    #[error("credential IO error: {0}")]
    Io(String),
}

/// The tool a meeting receipt is minted under. The node matches it exactly.
pub const MEETING_ATTEST_TOOL: &str = "note67.meeting.attest";

fn path(dir: &Path) -> PathBuf {
    dir.join(CREDENTIAL_FILE)
}

/// Store a credential, if it belongs to this installation and permits anything.
///
/// The subject check is the one that matters. A credential naming another
/// device would let this app present authority it cannot sign for — every
/// receipt it produced would be signed by a key the credential does not name,
/// and would fail verification at whoever tried to rely on it.
pub fn install(dir: &Path, our_did: &str, json: &str) -> Result<Credential, CredentialError> {
    let credential: Credential =
        serde_json::from_str(json).map_err(|e| CredentialError::Malformed(e.to_string()))?;

    if credential.subject_did != our_did {
        return Err(CredentialError::WrongSubject {
            subject: credential.subject_did,
            ours: our_did.to_string(),
        });
    }

    // Refused rather than stored. An empty tools list is a credential that
    // authorises nothing, and installing it would report this device as
    // enrolled while every attempt to attest was denied — a failure that would
    // be blamed on the node rather than on the mint.
    if credential.authority_scope.tools.is_empty() {
        return Err(CredentialError::NoTools {
            expected: MEETING_ATTEST_TOOL,
        });
    }

    std::fs::create_dir_all(dir).map_err(|e| CredentialError::Io(e.to_string()))?;
    // Written as received, not re-serialised from the struct: the signature
    // covers the issuer's bytes, and a round trip through our own types could
    // reorder or drop something and quietly invalidate it.
    std::fs::write(path(dir), json).map_err(|e| CredentialError::Io(e.to_string()))?;

    Ok(credential)
}

/// The credential exactly as the issuer wrote it.
///
/// The only safe input for anything that has to act under it. The struct above
/// models a subset — enough to check and display — and round-tripping through
/// it silently drops every field it does not name, including `intent_id` and
/// the objectives. A credential reassembled that way is not the credential that
/// was signed, and the node refuses it.
pub fn load_raw(dir: &Path) -> Option<String> {
    std::fs::read_to_string(path(dir)).ok()
}

/// The installed credential, if there is one.
pub fn load(dir: &Path) -> Option<Credential> {
    let json = std::fs::read_to_string(path(dir)).ok()?;
    serde_json::from_str(&json).ok()
}

/// Forget the credential. The identity stays: the key is this device's, and
/// removing it would orphan every receipt already anchored to the DID.
pub fn remove(dir: &Path) -> Result<(), CredentialError> {
    match std::fs::remove_file(path(dir)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(CredentialError::Io(e.to_string())),
    }
}

/// Whether a credential is currently usable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Standing {
    Active,
    /// Past its expiry. Kept and shown rather than deleted — a receipt minted
    /// while it was valid is still valid, and silently discarding the
    /// credential would lose the record of what authorised them.
    Expired,
    /// Cannot attest meetings, because the tool is not in its scope.
    NotForMeetings,
}

pub fn standing(credential: &Credential, now_ms: i64) -> Standing {
    if credential.expires_at.physical_ms <= now_ms {
        return Standing::Expired;
    }
    if !credential
        .authority_scope
        .tools
        .iter()
        .any(|t| t == MEETING_ATTEST_TOOL)
    {
        return Standing::NotForMeetings;
    }
    Standing::Active
}

/// Milliseconds since the epoch, as the credential's timestamps are expressed.
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = "did:exo:8MejPR8VCNWD3WYchtr2EAMSU3qz94FEdC8SwknGPjrE";

    /// A real credential, as the AVC App Registry issued it on 2026-08-12.
    ///
    /// Kept verbatim rather than hand-built: the struct this module first
    /// modelled was a different, registry-internal shape, and every real
    /// credential was rejected with "invalid type: map, expected a sequence".
    /// A fixture written from the actual output is what would have caught it.
    fn real_credential(subject: &str, tools: &str, expires_ms: i64) -> String {
        format!(
            r#"{{
              "authority_chain": null,
              "authority_scope": {{
                "counterparties": [],
                "data_classes": ["Internal"],
                "jurisdictions": ["US"],
                "permissions": ["Read", "Write"],
                "tools": [{tools}]
              }},
              "consent_refs": [],
              "constraints": {{
                "allowed_time_window": null,
                "human_approval_required": false,
                "max_delegation_depth": 0
              }},
              "created_at": {{ "logical": 0, "physical_ms": 1786581953043 }},
              "delegated_intent": {{
                "allowed_objectives": [],
                "autonomy_level": "ExecuteWithHumanApproval",
                "delegation_allowed": false,
                "intent_id": [202, 60, 90, 141],
                "prohibited_objectives": [],
                "purpose": "Attest that meeting transcripts recorded on this device are unchanged since recording."
              }},
              "expires_at": {{ "logical": 0, "physical_ms": {expires_ms} }},
              "holder_did": null,
              "issuer_did": "did:exo:8EVGmqLo15JEnrbcrLo9r84qX1mtrVeBdPjHLUtb1sXX",
              "parent_avc_id": null,
              "policy_refs": [],
              "principal_did": "did:exo:8EVGmqLo15JEnrbcrLo9r84qX1mtrVeBdPjHLUtb1sXX",
              "schema_version": 1,
              "signature": {{ "Ed25519": [7, 253, 228, 51] }},
              "subject_did": "{subject}",
              "subject_kind": {{ "Service": {{ "service_id": "note67" }} }}
            }}"#
        )
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("note67-cred-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const FUTURE: i64 = 1_818_117_953_043;
    const PAST: i64 = 1_700_000_000_000;
    const NOW: i64 = 1_786_600_000_000;

    #[test]
    fn a_real_credential_parses() {
        // The regression. authority_scope is an object and delegated_intent is
        // an object; modelling either as a list rejects every real credential.
        let dir = temp();
        let c = install(
            &dir,
            OURS,
            &real_credential(OURS, "\"note67.meeting.attest\"", FUTURE),
        )
        .unwrap();
        assert_eq!(c.subject_did, OURS);
        assert_eq!(c.authority_scope.tools, vec![MEETING_ATTEST_TOOL]);
        assert_eq!(c.authority_scope.permissions, vec!["Read", "Write"]);
        assert_eq!(c.delegated_intent.autonomy_level, "ExecuteWithHumanApproval");
        assert_eq!(c.expires_at.physical_ms, FUTURE);
    }

    #[test]
    fn a_credential_with_no_tools_is_refused() {
        // What the registry produced when the custom scope was typed but not
        // added. Stored, it would report this device as enrolled while every
        // attempt to attest was denied — and the node would get the blame.
        let dir = temp();
        let err = install(&dir, OURS, &real_credential(OURS, "", FUTURE)).unwrap_err();
        assert!(matches!(err, CredentialError::NoTools { .. }));
        assert!(err.to_string().contains(MEETING_ATTEST_TOOL), "{err}");
        assert!(load(&dir).is_none());
    }

    #[test]
    fn a_credential_for_another_device_is_refused() {
        // The check that matters. Installed, this app would present authority
        // it cannot sign for, and every receipt it produced would fail
        // verification at whoever tried to rely on it.
        let dir = temp();
        let err = install(
            &dir,
            OURS,
            &real_credential("did:exo:somebodyelse", "\"note67.meeting.attest\"", FUTURE),
        )
        .unwrap_err();
        assert!(matches!(err, CredentialError::WrongSubject { .. }));
        assert!(load(&dir).is_none(), "a refused credential was still stored");
    }

    #[test]
    fn the_error_names_both_dids() {
        // Pasting the wrong file is the likely mistake, and "wrong credential"
        // alone gives nobody a way to see which one they wanted.
        let dir = temp();
        let err = install(
            &dir,
            OURS,
            &real_credential("did:exo:somebodyelse", "\"note67.meeting.attest\"", FUTURE),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("did:exo:somebodyelse"), "{err}");
        assert!(err.contains(OURS), "{err}");
    }

    #[test]
    fn something_that_is_not_a_credential_is_refused() {
        let dir = temp();
        assert!(matches!(
            install(&dir, OURS, "{\"hello\":\"world\"}").unwrap_err(),
            CredentialError::Malformed(_)
        ));
        assert!(matches!(
            install(&dir, OURS, "not json at all").unwrap_err(),
            CredentialError::Malformed(_)
        ));
    }

    #[test]
    fn the_stored_bytes_are_the_issuers_bytes() {
        // The signature covers what the issuer serialised. Re-serialising from
        // our own struct could reorder or drop a field and quietly invalidate
        // it, and nothing would notice until a verification failed elsewhere.
        let dir = temp();
        let json = real_credential(OURS, "\"note67.meeting.attest\"", FUTURE);
        install(&dir, OURS, &json).unwrap();
        assert_eq!(std::fs::read_to_string(path(&dir)).unwrap(), json);
    }

    #[test]
    fn fields_this_app_does_not_model_survive() {
        // schema_version, consent_refs, policy_refs, subject_kind and the rest
        // are the node's business, and dropping them would invalidate the
        // signature over them.
        let dir = temp();
        let c = install(
            &dir,
            OURS,
            &real_credential(OURS, "\"note67.meeting.attest\"", FUTURE),
        )
        .unwrap();
        assert_eq!(c.rest.get("schema_version"), Some(&serde_json::json!(1)));
        assert!(c.rest.contains_key("subject_kind"));
        assert!(c.rest.contains_key("policy_refs"));
    }

    #[test]
    fn the_raw_bytes_come_back_byte_for_byte() {
        // What anything acting under the credential must use. The modelled
        // struct drops every field it does not name — intent_id, the
        // objectives — and a credential rebuilt from it is not the one that
        // was signed.
        let dir = temp();
        let json = real_credential(OURS, "\"note67.meeting.attest\"", FUTURE);
        install(&dir, OURS, &json).unwrap();
        assert_eq!(load_raw(&dir).unwrap(), json);
    }

    #[test]
    fn the_modelled_struct_is_lossy_and_must_not_be_re_serialised() {
        // Pinning the reason `load_raw` exists. If this ever round-trips
        // cleanly the comment above is wrong, but relying on it would still be
        // relying on a subset staying complete.
        let dir = temp();
        let json = real_credential(OURS, "\"note67.meeting.attest\"", FUTURE);
        let c = install(&dir, OURS, &json).unwrap();
        let round_tripped = serde_json::to_value(&c).unwrap();
        assert!(
            round_tripped["delegated_intent"].get("intent_id").is_none(),
            "the modelled struct kept intent_id; check whether load_raw is still needed"
        );
    }

    #[test]
    fn no_credential_reads_as_absent_rather_than_failing() {
        assert!(load(&temp()).is_none());
    }

    #[test]
    fn an_unexpired_credential_naming_the_tool_is_active() {
        let c: Credential =
            serde_json::from_str(&real_credential(OURS, "\"note67.meeting.attest\"", FUTURE))
                .unwrap();
        assert_eq!(standing(&c, NOW), Standing::Active);
    }

    #[test]
    fn an_expired_credential_is_reported_not_hidden() {
        // Receipts minted while it was valid are still valid, so discarding it
        // would lose the record of what authorised them.
        let c: Credential =
            serde_json::from_str(&real_credential(OURS, "\"note67.meeting.attest\"", PAST))
                .unwrap();
        assert_eq!(standing(&c, NOW), Standing::Expired);
    }

    #[test]
    fn a_credential_scoped_to_other_tools_cannot_attest_meetings() {
        // The node matches the tool exactly, so a credential carrying only
        // someone else's scope is valid and useless here. Saying which is more
        // use than reporting it enrolled.
        let c: Credential =
            serde_json::from_str(&real_credential(OURS, "\"archon.run\"", FUTURE)).unwrap();
        assert_eq!(standing(&c, NOW), Standing::NotForMeetings);
    }

    #[test]
    fn removing_a_credential_that_is_not_there_is_not_an_error() {
        assert!(remove(&temp()).is_ok());
    }

    #[test]
    fn removing_a_credential_leaves_the_identity_alone() {
        // The key is this device's. Removing it would orphan every receipt
        // already anchored to the DID.
        let dir = temp();
        install(
            &dir,
            OURS,
            &real_credential(OURS, "\"note67.meeting.attest\"", FUTURE),
        )
        .unwrap();
        std::fs::write(dir.join("identity.key"), [0u8; 32]).unwrap();

        remove(&dir).unwrap();
        assert!(load(&dir).is_none());
        assert!(dir.join("identity.key").exists());
    }

}
