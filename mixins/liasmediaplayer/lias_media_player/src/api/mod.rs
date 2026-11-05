//! Public Rust addon API v4. See `docs/addons.md` and the bundled addon example.
//!
//! Register once during initialization. Source matching runs on the client thread and
//! must be fast and nonblocking. Resolvers run on workers and must honor cancellation.
//! Events use bounded, nonblocking subscriptions; UI callbacks never call addon code.

pub(crate) mod media;
mod registry;
pub use crate::core::{
    player::{PlaybackState, PlayerCommand, PlayerSnapshot},
    process::Cancellation,
    resolver::{ResolveOptions, ResolvedMedia, StreamUrl},
    sources::{LinkSource, MediaKind, MediaRef},
};
pub use crate::ui::theme::{ThemeRole, ThemeSpec};
pub(crate) use media::publish;
pub use media::{
    EventSubscription, MediaEvent, MediaHandle, MediaSnapshot, WindowCommand, active_media, open,
    subscribe,
};
pub use registry::{AddonContext, MediaPlayerAddon, RegisterError, register_addon};
pub(crate) use registry::{resolvers, sources};
use std::io;
pub use url::Url;

pub const API_VERSION: u32 = 4;

/// Worker request. Resolvers are tried in registration order before built-ins.
#[derive(Clone, Copy, Debug)]
pub struct ResolveRequest<'a> {
    pub url: &'a str,
    pub kind: MediaKind,
    pub options: ResolveOptions,
}
/// Return `None` to decline; `Some(Err(_))` is a terminal failure, not a fallback.
/// Implementations must be thread-safe, bounded, and check `cancel` during I/O.
pub trait Resolver: Send + Sync {
    fn id(&self) -> &str;
    fn resolve(
        &self,
        request: ResolveRequest<'_>,
        cancel: &Cancellation,
    ) -> Option<io::Result<ResolvedMedia>>;
}

/// An extra title-bar button. Clicking emits `MediaEvent::WindowAction`.
/// At most four addon buttons can be registered globally. Labels are one character.
#[derive(Clone, Debug)]
pub struct WindowAction {
    pub id: String,
    pub label: String,
}

pub fn themes() -> Vec<ThemeSpec> {
    let mut themes = crate::ui::theme::builtins().to_vec();
    themes.extend(registry::themes());
    themes
}
pub fn window_actions() -> Vec<WindowAction> {
    registry::window_actions()
}
