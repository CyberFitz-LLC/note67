//! DID derivation, exactly as exochain does it.
//!
//! Lives here rather than in the app because the sync service verifies that a
//! device's registered DID matches the public key it presents. Two
//! implementations of the same three lines is precisely the divergence a
//! canonical form exists to prevent, so there is one.

/// Derive a DID from an Ed25519 public key.
///
/// From exochain's `exo-identity/src/did.rs`:
///
/// ```text
/// let hash = blake3::hash(public_key.as_bytes());
/// let encoded = bs58::encode(hash.as_bytes()).into_string();
/// Did::new(&format!("did:exo:{encoded}"))
/// ```
///
/// Reimplemented rather than linked because `exochain-core` 0.2.3 does not
/// build from crates.io — it depends on `ml-dsa 0.1.0-rc.7` with default
/// features, which pins a pre-release `pkcs8` while cargo resolves the released
/// 0.11.0. Filed as exochain#812. When that is fixed this should become a call
/// to `did_from_public_key`, and `the_derivation_matches_exochains_spec` below
/// is what proves the swap changed no DID.
///
/// The whole hash is encoded, never a prefix. A truncated suffix is the footgun
/// the onboarding docs call out, and the node rejects such a DID at emit time.
pub fn derive_did(public_key: &[u8; 32]) -> String {
    let hash = blake3::hash(public_key);
    format!("did:exo:{}", bs58::encode(hash.as_bytes()).into_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ed25519 public key for the all-zero seed — a published test vector.
    const KNOWN_PUBLIC_KEY: [u8; 32] = [
        0x3b, 0x6a, 0x27, 0xbc, 0xce, 0xb6, 0xa4, 0x2d, 0x62, 0xa3, 0xa8, 0xd0, 0x2a, 0x6f, 0x0d,
        0x73, 0x65, 0x32, 0x15, 0x77, 0x1d, 0xe2, 0x43, 0xa6, 0x3a, 0xc0, 0x48, 0xa1, 0x8b, 0x59,
        0xda, 0x29,
    ];

    #[test]
    fn the_derivation_matches_exochains_spec() {
        let did = derive_did(&KNOWN_PUBLIC_KEY);
        let suffix = did.strip_prefix("did:exo:").expect("did:exo: prefix");

        // base58 of 32 bytes is 43 or 44 characters. Shorter means the hash was
        // cut somewhere.
        assert!(
            (43..=44).contains(&suffix.len()),
            "unexpected length {}: {did}",
            suffix.len()
        );

        // Decodes back to exactly the blake3 digest of the key, which is the
        // property the node checks.
        let decoded = bs58::decode(suffix).into_vec().expect("suffix is base58");
        assert_eq!(decoded, blake3::hash(&KNOWN_PUBLIC_KEY).as_bytes().to_vec());
    }

    #[test]
    fn the_derivation_is_stable() {
        // Pinned literally, and computed rather than assumed. If this value
        // ever changes, every DID already registered stops matching its key and
        // every receipt anchored to one becomes unverifiable.
        assert_eq!(
            derive_did(&KNOWN_PUBLIC_KEY),
            "did:exo:6WsMUE5qST6hrvvvbNRDxTGe9xtQwW5ig5try9u487Rw"
        );
    }

    #[test]
    fn different_keys_get_different_dids() {
        assert_ne!(derive_did(&[1u8; 32]), derive_did(&[2u8; 32]));
    }
}
