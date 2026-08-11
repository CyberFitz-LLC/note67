//! Syncing with the archive.
//!
//! The app works signed out. Everything here is inert until someone signs in:
//! recording, transcription and the local transcript chain never need a token,
//! and a note exists before it has an owner.

pub mod auth;
pub mod payload;
pub mod state;
