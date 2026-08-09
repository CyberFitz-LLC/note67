//! ExoChain integration.
//!
//! Scope and rationale live in `docs/exochain/` — SCOPING.md for what is
//! governed and why, PRD-meeting-receipts.md for the design, DECISIONS.md for
//! the choices behind it.
//!
//! Nothing here talks to a node yet: identity is derived and held locally, and
//! is useful on its own for chaining transcript versions before any credential
//! exists.

pub mod identity;

pub use identity::{Identity, IdentityError};
