//! Segmenta CLI library: the JSON-RPC client, the renderers and the command surface.
//!
//! The crate ships two identical binaries, `segmenta-cli` and `segmenta`, so both the
//! documented OpenSpec name and the short name work after installing the .deb.

pub mod cli;
pub mod client;
pub mod render;
