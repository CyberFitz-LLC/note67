//! Exposing this installation's ExoChain identity.
//!
//! The identity is created on first use and never leaves the machine — only the
//! DID and public key do. A credential names the key that will sign receipts, so
//! until the app can show its own DID there is nothing for an issuer to make a
//! credential *about*.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use crate::exochain::credential::{self, Credential, Standing};
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
    pub id: String,
    pub issuer_did: String,
    pub issued_at: String,
    pub expires_at: String,
    pub authority_scope: Vec<String>,
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
            .is_some_and(|c| credential::standing(c, &chrono::Utc::now().to_rfc3339()) == Standing::Active),
        credential: credential.map(|c| CredentialView {
            standing: credential::standing(&c, &chrono::Utc::now().to_rfc3339()),
            id: c.id,
            issuer_did: c.issuer_did,
            issued_at: c.issued_at,
            expires_at: c.expires_at,
            authority_scope: c.authority_scope,
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
