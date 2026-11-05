//! Lia's Media Player for Horizon: the mod's own crate.
//!
//! Horizon compiles this crate once as a workspace member that `horizon-client` depends on
//! (`[[crate]]` in `horizon-mixin.toml`). It has no Horizon imports: the glue that talks to
//! `Game` lives in the bundle's `bridge.rs` support module, so everything here builds and
//! tests with plain `cargo test`.
//!
//! M0–M7 provide playback, full screens, chat integration and the public addon API.
//! Press M to interact with the floating media windows (README §6).

pub mod api;
pub use api::register_addon;

pub mod command;
pub mod core;
pub mod platform;
pub mod runtime;
pub mod ui;

pub use command::{ParseError, ShowCommand, ShowKind};
pub use runtime::{Badge, Runtime};

/// The mod's id: its folder in `horizon-mods/.horizon-mod-data/`, its keys' prefix and its
/// row in Options > Mods. Lowercase letters, digits and `_` only (the registry's rule).
pub const ID: &str = "liasmediaplayer";
/// The name Options > Mods shows.
pub const NAME: &str = "Lia's Media Player";
/// The mod's version, kept in step with `horizon-mixin.toml` and `horizon-native.json`.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// The description Options > Mods shows.
pub const DESCRIPTION: &str =
    "Watch YouTube, Twitch, videos, GIFs and images, and listen to audio, inside Horizon.";

/// The client commands the mod owns, by name and without their slash (`native_commands`).
pub const COMMANDS: &[&str] = &[command::NAME];

/// `Name 1.0`, as Options > Mods titles a native mod.
pub fn title() -> String {
    format!("{NAME} {VERSION}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_follows_the_registry_rule() {
        assert!(!ID.is_empty());
        assert!(
            ID.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        );
    }

    #[test]
    fn title_is_name_and_version() {
        assert_eq!(title(), "Lia's Media Player 4.0.0");
    }

    #[test]
    fn owns_the_show_command() {
        assert_eq!(COMMANDS, &["show"]);
    }
}
