//! Byte-exact formats shared between the Note67 desktop app and its sync
//! service.
//!
//! The service does not trust what a client submits: it recomputes a transcript
//! version's content hash and rejects a mismatch, and it verifies that a
//! device's DID matches the public key it presents. Both checks are only
//! meaningful if the two sides produce identical bytes, which they cannot be
//! relied on to do from a written specification alone — so the code lives once,
//! here, and both depend on it.
//!
//! Nothing that is not a shared format belongs in this crate.

pub mod did;
pub mod transcript;

pub use did::derive_did;
pub use transcript::{
    canonical_bytes, content_hash, next_version, verify_chain, CanonicalSegment, ChainError,
    ImportSource, Origin, Reason, TranscriptVersion, SERIALIZATION_V1,
};
