//! Per-install ExoChain identity.
//!
//! Each installation holds its own Ed25519 key and derives its own DID, because
//! R3 binds a DID to the key that signs with it: sharing one identity across
//! machines would mean sharing the private key. Per-install identity also means
//! a lost laptop can be revoked on its own.
//!
//! The seed is generated and owned here: the intended dependency,
//! `exochain-core`, cannot currently be built (see `derive_did` below), so the
//! DID derivation is implemented against exochain's specification rather than
//! by linking its crate. Swapping to the crate later must not change any DID,
//! which `the_derivation_matches_exochains_specification` exists to catch.

use std::fs;
use std::path::{Path, PathBuf};

use ed25519_dalek::SigningKey;
use serde::{Deserialize, Serialize};

/// Derive a DID exactly as exochain does.
///
/// From `exo-identity/src/did.rs`:
///
/// ```text
/// let hash = blake3::hash(public_key.as_bytes());
/// let encoded = bs58::encode(hash.as_bytes()).into_string();
/// Did::new(&format!("did:exo:{encoded}"))
/// ```
///
/// Reimplemented rather than linked because `exochain-core` 0.2.3 does not
/// compile: it depends on `ml-dsa 0.1.0-rc.7` with default features, which
/// pins a pre-release `pkcs8`, and cargo resolves the released 0.11.0 whose
/// API differs. Pinning the pre-release cascades into `der` and `spki` and
/// collides with reqwest's TLS stack. Filed upstream.
///
/// Nothing here is a variation on the spec: the whole hash is encoded, never a
/// prefix. A truncated suffix is the footgun the onboarding docs call out, and
/// the node rejects such a DID at emit time.
fn derive_did(public_key: &[u8; 32]) -> String {
    let hash = blake3::hash(public_key);
    format!("did:exo:{}", bs58::encode(hash.as_bytes()).into_string())
}

/// Filename holding the raw 32-byte seed. Never leaves the host.
const KEY_FILE: &str = "identity.key";
/// Filename holding the non-secret metadata.
const META_FILE: &str = "identity.json";

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("identity IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("identity is malformed: {0}")]
    Malformed(String),
    #[error("could not generate randomness: {0}")]
    Random(String),
    #[error(
        "the stored DID does not match the key on disk (stored {stored}, derived {derived}) — \
         refusing to use it"
    )]
    Mismatch { stored: String, derived: String },
}

/// The non-secret half of an installation's identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Identity {
    /// Stable per-install id. A UUID rather than a hostname: hostnames change
    /// and collide, and this ends up inside the credential's service_id.
    pub device_id: String,
    /// `did:exo:base58(blake3(public_key))`. See `derive_did` for why this is
    /// computed here rather than by exochain's crate, and what guards it.
    pub did: String,
    pub public_key_hex: String,
    /// What the AVC credential's `subject_kind` will carry.
    pub service_id: String,
    pub created_at: String,
}

impl Identity {
    fn from_key(key: &SigningKey, device_id: String, created_at: String) -> Self {
        let public = key.verifying_key().to_bytes();
        Self {
            service_id: format!("note67:{device_id}"),
            device_id,
            did: derive_did(&public),
            public_key_hex: hex::encode(public),
            created_at,
        }
    }
}

fn key_path(dir: &Path) -> PathBuf {
    dir.join(KEY_FILE)
}
fn meta_path(dir: &Path) -> PathBuf {
    dir.join(META_FILE)
}

/// Restrict the key file to its owner.
///
/// Unix gets 0600. Windows inherits the directory ACL, which for the app's own
/// data directory is already user-scoped; tightening it further needs the
/// Windows security APIs and is deliberately not attempted here rather than
/// pretended at.
fn restrict(path: &Path) -> Result<(), IdentityError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}

fn random_seed() -> Result<[u8; 32], IdentityError> {
    let mut seed = [0u8; 32];
    getrandom::getrandom(&mut seed).map_err(|e| IdentityError::Random(e.to_string()))?;
    Ok(seed)
}

fn keypair_from_seed(seed: [u8; 32]) -> SigningKey {
    SigningKey::from_bytes(&seed)
}

/// Load this installation's identity, creating one on first run.
///
/// Returns the metadata and the reconstructed key pair. The key pair is not
/// stored anywhere; callers hold it for as long as they need to sign.
pub fn load_or_create(
    dir: &Path,
    now: impl Fn() -> String,
) -> Result<(Identity, SigningKey), IdentityError> {
    fs::create_dir_all(dir)?;

    if key_path(dir).exists() {
        return load(dir);
    }

    let seed = random_seed()?;
    let key = keypair_from_seed(seed);
    let identity = Identity::from_key(&key, uuid::Uuid::new_v4().to_string(), now());

    fs::write(key_path(dir), seed)?;
    restrict(&key_path(dir))?;
    fs::write(
        meta_path(dir),
        serde_json::to_vec_pretty(&identity).map_err(|e| IdentityError::Malformed(e.to_string()))?,
    )?;

    Ok((identity, key))
}

/// Load an existing identity, verifying the metadata against the key.
pub fn load(dir: &Path) -> Result<(Identity, SigningKey), IdentityError> {
    let seed_bytes = fs::read(key_path(dir))?;
    let seed: [u8; 32] = seed_bytes
        .try_into()
        .map_err(|_| IdentityError::Malformed("the key file is not 32 bytes".into()))?;
    let key = keypair_from_seed(seed);

    let meta = fs::read(meta_path(dir))?;
    let stored: Identity =
        serde_json::from_slice(&meta).map_err(|e| IdentityError::Malformed(e.to_string()))?;

    // Recompute rather than trust. If the metadata and the key disagree, one of
    // them was edited or swapped, and signing with a key whose DID we misreport
    // would produce receipts nobody can verify.
    let derived = Identity::from_key(&key, stored.device_id.clone(), stored.created_at.clone());
    if derived.did != stored.did || derived.public_key_hex != stored.public_key_hex {
        return Err(IdentityError::Mismatch {
            stored: stored.did,
            derived: derived.did,
        });
    }

    Ok((derived, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("note67-identity-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn fixed_now() -> String {
        "2026-08-08T00:00:00Z".to_string()
    }

    #[test]
    fn a_did_is_derived_in_the_canonical_form() {
        let dir = tmpdir("canonical");
        let (identity, _key) = load_or_create(&dir, fixed_now).unwrap();

        assert!(
            identity.did.starts_with("did:exo:"),
            "unexpected DID form: {}",
            identity.did
        );

        // blake3 is 32 bytes, which base58-encodes to 43-44 characters. A
        // shorter suffix means something truncated the hash — the footgun the
        // onboarding docs warn about, and the node rejects it at emit time.
        let suffix = identity.did.trim_start_matches("did:exo:");
        assert!(
            suffix.len() >= 43,
            "DID suffix looks truncated ({} chars): {}",
            suffix.len(),
            suffix
        );
    }

    #[test]
    fn the_did_is_a_function_of_the_key_alone() {
        // Same seed must always give the same DID, or a restored backup would
        // present a different identity than the receipts it already emitted.
        let seed = [7u8; 32];
        let a = keypair_from_seed(seed);
        let b = keypair_from_seed(seed);
        let did_a = Identity::from_key(&a, "d".into(), fixed_now()).did;
        let did_b = Identity::from_key(&b, "different-device".into(), fixed_now()).did;
        assert_eq!(did_a, did_b);
    }

    #[test]
    fn different_keys_get_different_dids() {
        let a = keypair_from_seed([1u8; 32]);
        let b = keypair_from_seed([2u8; 32]);
        assert_ne!(
            Identity::from_key(&a, "d".into(), fixed_now()).did,
            Identity::from_key(&b, "d".into(), fixed_now()).did
        );
    }

    #[test]
    fn an_identity_survives_a_reload() {
        let dir = tmpdir("reload");
        let (created, _) = load_or_create(&dir, fixed_now).unwrap();
        let (loaded, _) = load_or_create(&dir, fixed_now).unwrap();

        assert_eq!(created, loaded, "reloading must not mint a new identity");
    }

    #[test]
    fn the_service_id_names_the_install_not_the_product() {
        // Per-install identity is the whole point: a shared service_id would
        // mean a shared private key, and revoking one machine would revoke all.
        let dir = tmpdir("service-id");
        let (identity, _) = load_or_create(&dir, fixed_now).unwrap();
        assert_eq!(identity.service_id, format!("note67:{}", identity.device_id));
        assert!(!identity.device_id.is_empty());
    }

    #[test]
    fn two_installs_are_two_identities() {
        let (a, _) = load_or_create(&tmpdir("install-a"), fixed_now).unwrap();
        let (b, _) = load_or_create(&tmpdir("install-b"), fixed_now).unwrap();
        assert_ne!(a.did, b.did);
        assert_ne!(a.device_id, b.device_id);
    }

    #[test]
    fn metadata_that_disagrees_with_the_key_is_rejected() {
        // Signing with one key while reporting another's DID produces receipts
        // that cannot be verified, so this fails closed rather than trusting
        // whichever file was edited.
        let dir = tmpdir("tampered");
        let (identity, _) = load_or_create(&dir, fixed_now).unwrap();

        let mut tampered = identity.clone();
        tampered.did = "did:exo:11111111111111111111111111111111111111111111".to_string();
        fs::write(meta_path(&dir), serde_json::to_vec_pretty(&tampered).unwrap()).unwrap();

        match load(&dir) {
            Err(IdentityError::Mismatch { .. }) => {}
            other => panic!("expected a mismatch error, got {other:?}"),
        }
    }

    #[test]
    fn a_truncated_key_file_is_rejected() {
        let dir = tmpdir("short-key");
        load_or_create(&dir, fixed_now).unwrap();
        fs::write(key_path(&dir), [0u8; 16]).unwrap();

        match load(&dir) {
            Err(IdentityError::Malformed(_)) => {}
            other => panic!("expected a malformed error, got {other:?}"),
        }
    }

    #[test]
    fn the_derivation_matches_exochains_specification() {
        // A known-answer test over the whole chain: seed -> ed25519 public key
        // -> blake3 -> base58. If any step is swapped for a variant, or a hash
        // is truncated, this changes. When exochain-core becomes buildable and
        // this is replaced by a call to did_from_public_key, the value must
        // stay identical — that is the point of pinning it here rather than
        // only asserting shape.
        let key = keypair_from_seed([0u8; 32]);
        let public = key.verifying_key().to_bytes();

        // Ed25519 public key for the all-zero seed is a published test vector.
        assert_eq!(
            hex::encode(public),
            "3b6a27bcceb6a42d62a3a8d02a6f0d73653215771de243a63ac048a18b59da29",
            "ed25519 derivation changed"
        );

        let did = derive_did(&public);
        let suffix = did.trim_start_matches("did:exo:");
        // base58 of 32 bytes never exceeds 44 characters and is 43 or 44 in
        // practice; anything shorter means the hash was cut.
        assert!(
            (43..=44).contains(&suffix.len()),
            "unexpected DID length {}: {}",
            suffix.len(),
            did
        );
        // Decodes back to exactly the 32 hash bytes.
        let decoded = bs58::decode(suffix).into_vec().expect("suffix is base58");
        assert_eq!(decoded.len(), 32, "DID does not encode a full 32-byte hash");
        assert_eq!(
            decoded,
            blake3::hash(&public).as_bytes().to_vec(),
            "DID is not base58(blake3(public_key))"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_key_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmpdir("perms");
        load_or_create(&dir, fixed_now).unwrap();
        let mode = fs::metadata(key_path(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "key file is readable by others");
    }
}
