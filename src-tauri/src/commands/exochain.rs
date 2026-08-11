//! Exposing this installation's ExoChain identity.
//!
//! The identity is created on first use and never leaves the machine — only the
//! DID and public key do. A credential names the key that will sign receipts, so
//! until the app can show its own DID there is nothing for an issuer to make a
//! credential *about*.

use serde::Serialize;
use tauri::{AppHandle, Manager};

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
    /// True once a credential naming this DID has been installed.
    ///
    /// Always false today: nothing issues one yet. Present so the UI can say
    /// "not enrolled" rather than implying attestation is happening when it is
    /// not — the distinction the receipts are supposed to make.
    pub enrolled: bool,
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
    Ok(view(identity))
}

fn view(identity: Identity) -> IdentityView {
    IdentityView {
        did: identity.did,
        public_key_hex: identity.public_key_hex,
        device_id: identity.device_id,
        service_id: identity.service_id,
        created_at: identity.created_at,
        // Enrollment is not built. Reporting anything else here would be the
        // app claiming a credential it does not hold.
        enrolled: false,
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
    fn an_identity_reports_itself_unenrolled() {
        // Nothing issues a credential yet. Saying otherwise would have the app
        // claim attestation it cannot perform — the exact distinction receipts
        // exist to make.
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
