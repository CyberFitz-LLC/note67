//! Exposing this installation's ExoChain identity.
//!
//! The identity is created on first use and never leaves the machine — only the
//! DID and public key do. A credential names the key that will sign receipts, so
//! until the app can show its own DID there is nothing for an issuer to make a
//! credential *about*.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::db::Database;
use crate::exochain::credential::{self, Credential, Standing};
use crate::exochain::emit::{self, Attestation};
use crate::exochain::identity::{self, Identity};

/// What the app knows about its own identity, and how far enrollment has got.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IdentityView {
    pub did: String,
    pub public_key_hex: String,
    pub device_id: String,
    pub service_id: String,
    pub created_at: String,
    /// True once a credential naming this DID has been installed and is usable.
    ///
    /// False for an expired or unreadable one too: the point of the flag is
    /// whether meetings can be attested right now, and a credential that
    /// cannot authorise anything should not read as enrolment.
    pub enrolled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential: Option<CredentialView>,
}

/// What the UI shows about an installed credential.
///
/// The signature is deliberately absent. It is of no use on screen and this
/// struct goes straight into the webview.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialView {
    pub issuer_did: String,
    /// Milliseconds since the epoch, as the credential expresses them.
    pub issued_at_ms: i64,
    pub expires_at_ms: i64,
    pub tools: Vec<String>,
    pub permissions: Vec<String>,
    pub data_classes: Vec<String>,
    pub purpose: String,
    pub standing: Standing,
}

fn identity_dir(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    app.path()
        .app_data_dir()
        .map_err(|e| format!("could not locate the app data directory: {e}"))
}

/// Read this installation's identity, creating it on first call.
///
/// Creating on read is deliberate. The identity is just a key and a name
/// derived from it; making the user perform an enrolment step to see one would
/// imply the app had asked permission from something, which it has not.
#[tauri::command]
pub fn get_exochain_identity(app: AppHandle) -> Result<IdentityView, String> {
    let dir = identity_dir(&app)?;
    let (identity, _key) = identity::load_or_create(&dir, || chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;
    Ok(view_with(identity, credential::load(&dir)))
}

/// Install a credential minted for this installation.
#[tauri::command]
pub fn install_exochain_credential(app: AppHandle, json: String) -> Result<IdentityView, String> {
    let dir = identity_dir(&app)?;
    let (identity, _key) = identity::load_or_create(&dir, || chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;

    let installed =
        credential::install(&dir, &identity.did, json.trim()).map_err(|e| e.to_string())?;
    Ok(view_with(identity, Some(installed)))
}

/// Remove the installed credential, keeping the identity.
#[tauri::command]
pub fn remove_exochain_credential(app: AppHandle) -> Result<IdentityView, String> {
    let dir = identity_dir(&app)?;
    credential::remove(&dir).map_err(|e| e.to_string())?;
    let (identity, _key) = identity::load_or_create(&dir, || chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;
    Ok(view_with(identity, None))
}

/// Where the node lives, and what to present to it.
///
/// Settings rather than constants so a different chain can be pointed at
/// without a rebuild — which is also how this gets tested against a local node
/// before anything reaches production.
const NODE_URL_KEY: &str = "exochain_node_url";
const NODE_TOKEN_KEY: &str = "exochain_node_token";
const DEFAULT_NODE_URL: &str = "https://exochain-production.up.railway.app";

/// Ask a node to attest this note's current transcript.
///

/// Attest one live-assistance session.
///
/// Returns the receipt hash, or a plain reason there is none. Deliberately a
/// `Result<String, String>` rather than an `Attestation`: the caller shows
/// either a receipt or why there is not one, and every way of having no receipt
/// reads the same to a user.
///
/// **Assistance runs either way.** A node that is unreachable, or a credential
/// that does not name this activity, leaves the session unattested and says so
/// — it does not silently pretend, and it does not stop a meeting because a
/// service is down.
pub async fn attest_assist_session(app: &AppHandle, note_id: &str) -> Result<String, String> {
    let dir = identity_dir(app)?;
    let (_identity, key) = identity::load_or_create(&dir, || chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;

    let Some(stored) = credential::load(&dir) else {
        return Err("no credential is installed, so the session is unattested".into());
    };
    if credential::standing(&stored, credential::now_ms()) != Standing::Active {
        return Err("the installed credential is not usable, so the session is unattested".into());
    }

    let raw = credential::load_raw(&dir)
        .ok_or("the stored credential could not be read, so the session is unattested")?;
    let parsed = serde_json::from_str(&raw)
        .map_err(|e| format!("the stored credential is not one this node would accept: {e}"))?;

    let request = emit::build_session_request(&parsed, &key, note_id, emit::now())?;

    let db = app.state::<Database>();
    let node_url = db
        .get_setting(NODE_URL_KEY)
        .ok()
        .flatten()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NODE_URL.to_string());
    let token = db.get_setting(NODE_TOKEN_KEY).ok().flatten();

    match emit::emit(&reqwest::Client::new(), &node_url, token.as_deref(), &request).await {
        Attestation::Attested { receipt_hash } => Ok(receipt_hash),
        Attestation::Denied { reason } => Err(format!(
            "the node refused to attest this session ({reason}). Live assistance needs \
             '{}' in the credential's authority scope.",
            emit::ASSIST_SESSION_TOOL
        )),
        Attestation::Pending { reason } => {
            Err(format!("the session is unattested — {reason}"))
        }
    }
}

/// Returns what the node said. A receipt is recorded only when one was minted:
/// an unreachable node leaves the transcript exactly as it was, which is the
/// whole reason recording never depends on the node.
#[tauri::command]
pub async fn attest_meeting(
    app: AppHandle,
    db: tauri::State<'_, Database>,
    note_id: String,
) -> Result<Attestation, String> {
    let dir = identity_dir(&app)?;
    let (identity, key) = identity::load_or_create(&dir, || chrono::Utc::now().to_rfc3339())
        .map_err(|e| e.to_string())?;

    let Some(stored) = credential::load(&dir) else {
        return Err("This installation has no credential yet, so nothing can be attested.".into());
    };
    if credential::standing(&stored, credential::now_ms()) != Standing::Active {
        return Err(
            "This installation's credential is not usable — see Settings, Meeting Receipts.".into(),
        );
    }

    // The version being attested, captured before the request: a receipt names
    // one content hash, and re-transcription during the round trip must not
    // move which version it lands on.
    let version = db
        .latest_transcript_version(&note_id)
        .map_err(|e| e.to_string())?
        .ok_or("This note has no transcript to attest.")?;

    // Parsed from the issuer's own bytes, never from the modelled struct: that
    // models a subset, so re-serialising it drops intent_id and the objectives
    // and produces something the node correctly refuses.
    let raw = credential::load_raw(&dir)
        .ok_or("The stored credential could not be read from disk.")?;
    let parsed = serde_json::from_str(&raw)
        .map_err(|e| format!("The stored credential is not one this node would accept: {e}"))?;

    let request = emit::build_request(&parsed, &key, &note_id, emit::now())?;

    let node_url = db
        .get_setting(NODE_URL_KEY)
        .ok()
        .flatten()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_NODE_URL.to_string());
    let token = db.get_setting(NODE_TOKEN_KEY).ok().flatten();

    let outcome = emit::emit(
        &reqwest::Client::new(),
        &node_url,
        token.as_deref(),
        &request,
    )
    .await;

    if let Attestation::Attested { receipt_hash } = &outcome {
        db.record_receipt(&note_id, version.version as i64, receipt_hash)
            .map_err(|e| e.to_string())?;
    }

    let _ = identity;
    Ok(outcome)
}

fn view(identity: Identity) -> IdentityView {
    view_with(identity, None)
}

fn view_with(identity: Identity, credential: Option<Credential>) -> IdentityView {
    IdentityView {
        did: identity.did,
        public_key_hex: identity.public_key_hex,
        device_id: identity.device_id,
        service_id: identity.service_id,
        created_at: identity.created_at,
        enrolled: credential
            .as_ref()
            .is_some_and(|c| credential::standing(c, credential::now_ms()) == Standing::Active),
        credential: credential.map(|c| CredentialView {
            standing: credential::standing(&c, credential::now_ms()),
            issuer_did: c.issuer_did,
            issued_at_ms: c.created_at.physical_ms,
            expires_at_ms: c.expires_at.physical_ms,
            tools: c.authority_scope.tools,
            permissions: c.authority_scope.permissions,
            data_classes: c.authority_scope.data_classes,
            purpose: c.delegated_intent.purpose,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_identity_view_never_carries_the_private_key() {
        // The seed is the one thing that must not leave the machine, and this
        // struct is serialized straight into the webview.
        let id = Identity {
            device_id: "d1".into(),
            did: "did:exo:abc".into(),
            public_key_hex: "3b6a".into(),
            service_id: "note67".into(),
            created_at: "2026-08-11T00:00:00Z".into(),
        };
        let json = serde_json::to_string(&view(id)).unwrap();
        assert!(json.contains("did:exo:abc"));
        for forbidden in ["seed", "privateKey", "secret", "signingKey"] {
            assert!(!json.contains(forbidden), "{forbidden} reached the view");
        }
    }

    #[test]
    fn an_identity_with_no_credential_reports_itself_unenrolled() {
        // Saying otherwise would have the app claim attestation it cannot
        // perform — the exact distinction receipts exist to make.
        let id = Identity {
            device_id: "d1".into(),
            did: "did:exo:abc".into(),
            public_key_hex: "3b6a".into(),
            service_id: "note67".into(),
            created_at: "2026-08-11T00:00:00Z".into(),
        };
        assert!(!view(id).enrolled);
    }
}
