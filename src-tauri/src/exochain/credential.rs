//! Holding the credential that says what this installation may do.
//!
//! The credential is minted elsewhere — the App Registry signs it with the
//! issuer key and registers it on the chain — and arrives here as JSON. This
//! module's job is to refuse the wrong one and to be honest about the state of
//! the right one.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const CREDENTIAL_FILE: &str = "credential.json";

/// A signed AVC, as the registry issues it.
///
/// Mirrors `exo_avc::SignedCredential`, deliberately by structure rather than
/// by dependency: the app only reads these fields, and taking the crate would
/// pull the pre-release crypto stack into a desktop build for no benefit.
/// Unknown fields are kept, because what is stored must stay byte-identical to
/// what was signed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Credential {
    pub id: String,
    pub issuer_did: String,
    pub subject_did: String,
    pub issued_at: String,
    pub expires_at: String,
    #[serde(default)]
    pub authority_scope: Vec<String>,
    #[serde(default)]
    pub delegated_intent: String,
    pub signature: String,
    /// Everything else the issuer sent, preserved verbatim.
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
    #[error("credential IO error: {0}")]
    Io(String),
}

fn path(dir: &Path) -> PathBuf {
    dir.join(CREDENTIAL_FILE)
}

/// Store a credential, if it belongs to this installation.
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

    std::fs::create_dir_all(dir).map_err(|e| CredentialError::Io(e.to_string()))?;
    // Written as received, not re-serialised from the struct: the signature
    // covers the issuer's bytes, and a round trip through our own types could
    // reorder or drop something and quietly invalidate it.
    std::fs::write(path(dir), json).map_err(|e| CredentialError::Io(e.to_string()))?;

    Ok(credential)
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
    /// The expiry could not be read, so its standing is unknown. Treated as
    /// unusable: assuming a credential is good because its date is unparseable
    /// is exactly the wrong default.
    Unreadable,
}

pub fn standing(credential: &Credential, now: &str) -> Standing {
    let (Ok(expires), Ok(now)) = (
        chrono::DateTime::parse_from_rfc3339(&credential.expires_at),
        chrono::DateTime::parse_from_rfc3339(now),
    ) else {
        return Standing::Unreadable;
    };

    if expires > now {
        Standing::Active
    } else {
        Standing::Expired
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OURS: &str = "did:exo:6WsMUE5qST6hrvvvbNRDxTGe9xtQwW5ig5try9u487Rw";

    fn json_for(subject: &str, expires: &str) -> String {
        serde_json::json!({
            "id": "abc123",
            "issuer_did": "did:exo:8EVGmqLo15JEnrbcrLo9r84qX1mtrVeBdPjHLUtb1sXX",
            "subject_did": subject,
            "issued_at": "2026-08-12T00:00:00Z",
            "expires_at": expires,
            "autonomy_level": 2,
            "delegated_intent": "Capture and transcribe meetings",
            "authority_scope": ["note67.meeting.attest"],
            "delegation_allowed": false,
            "constraints": {},
            "signature": "c2ln"
        })
        .to_string()
    }

    fn temp() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("note67-cred-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_credential_for_this_device_installs() {
        let dir = temp();
        let c = install(&dir, OURS, &json_for(OURS, "2027-08-12T00:00:00Z")).unwrap();
        assert_eq!(c.subject_did, OURS);
        assert_eq!(c.authority_scope, vec!["note67.meeting.attest"]);
        assert_eq!(load(&dir).unwrap(), c);
    }

    #[test]
    fn a_credential_for_another_device_is_refused() {
        // The check that matters. Installed, this app would present authority
        // it cannot sign for, and every receipt it produced would fail
        // verification at whoever tried to rely on it.
        let dir = temp();
        let err = install(&dir, OURS, &json_for("did:exo:somebodyelse", "2027-08-12T00:00:00Z"))
            .unwrap_err();
        assert!(matches!(err, CredentialError::WrongSubject { .. }));
        assert!(load(&dir).is_none(), "a refused credential was still stored");
    }

    #[test]
    fn the_error_names_both_dids() {
        // Pasting the wrong file is the likely mistake, and "wrong credential"
        // alone gives nobody a way to see which one they wanted.
        let dir = temp();
        let err = install(&dir, OURS, &json_for("did:exo:somebodyelse", "2027-01-01T00:00:00Z"))
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
        let json = json_for(OURS, "2027-08-12T00:00:00Z");
        install(&dir, OURS, &json).unwrap();
        assert_eq!(std::fs::read_to_string(path(&dir)).unwrap(), json);
    }

    #[test]
    fn fields_we_do_not_model_survive_a_round_trip() {
        let dir = temp();
        let mut value: serde_json::Value =
            serde_json::from_str(&json_for(OURS, "2027-08-12T00:00:00Z")).unwrap();
        value["something_new"] = serde_json::json!({"from": "a later schema"});
        let json = value.to_string();

        let c = install(&dir, OURS, &json).unwrap();
        assert_eq!(
            c.rest.get("something_new").and_then(|v| v.get("from")),
            Some(&serde_json::json!("a later schema"))
        );
    }

    #[test]
    fn no_credential_reads_as_absent_rather_than_failing() {
        assert!(load(&temp()).is_none());
    }

    #[test]
    fn an_unexpired_credential_is_active() {
        let c: Credential = serde_json::from_str(&json_for(OURS, "2027-08-12T00:00:00Z")).unwrap();
        assert_eq!(standing(&c, "2026-08-12T00:00:00Z"), Standing::Active);
    }

    #[test]
    fn an_expired_credential_is_reported_not_hidden() {
        // Receipts minted while it was valid are still valid, so discarding it
        // would lose the record of what authorised them.
        let c: Credential = serde_json::from_str(&json_for(OURS, "2026-01-01T00:00:00Z")).unwrap();
        assert_eq!(standing(&c, "2026-08-12T00:00:00Z"), Standing::Expired);
    }

    #[test]
    fn an_unreadable_expiry_is_not_assumed_good() {
        // Assuming a credential is valid because its date will not parse is
        // exactly the wrong default.
        let c: Credential = serde_json::from_str(&json_for(OURS, "whenever")).unwrap();
        assert_eq!(standing(&c, "2026-08-12T00:00:00Z"), Standing::Unreadable);
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
        install(&dir, OURS, &json_for(OURS, "2027-08-12T00:00:00Z")).unwrap();
        std::fs::write(dir.join("identity.key"), [0u8; 32]).unwrap();

        remove(&dir).unwrap();
        assert!(load(&dir).is_none());
        assert!(dir.join("identity.key").exists());
    }
}
