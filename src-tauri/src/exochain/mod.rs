//! ExoChain integration.
//!
//! Scope and rationale live in `docs/exochain/` — SCOPING.md for what is
//! governed and why, PRD-meeting-receipts.md for the design, DECISIONS.md for
//! the choices behind it.
//!
//! Nothing here talks to a node yet: identity is derived and held locally, and
//! the transcript chain is useful on its own before any credential exists.

pub mod credential;
pub mod emit;
pub mod identity;
pub mod verify;
pub mod vtt;

/// The canonical forms, which live in their own crate because the sync service
/// has to reproduce them byte for byte. Re-exported here so call sites read the
/// same either way.
pub use note67_canonical::transcript;

// Re-exported for the call sites that use them directly. Everything else stays
// reachable through its module — `exochain::identity::Identity`,
// `exochain::vtt::VttError`, `exochain::transcript::verify_chain` — rather than
// being surfaced here before anything consumes it.
pub use note67_canonical::{CanonicalSegment, ImportSource, Origin, Reason, TranscriptVersion};
pub use vtt::parse_vtt;
