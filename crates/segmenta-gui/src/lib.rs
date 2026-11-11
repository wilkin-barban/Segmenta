//! Segmenta GUI: GTK4 + Libadwaita client of the daemon's JSON-RPC socket.

pub mod add_dialog;
pub mod app;
pub mod client;
pub mod model;
pub mod prefs;
pub mod screenshots;
pub mod segment_view;
pub mod window;

// Re-exported so the integration tests can drive the toolkit without
// duplicating the dependency declarations of this crate.
pub use {glib, gtk, libadwaita as adw};
