use super::{LinkSource, Resolver, ThemeSpec, WindowAction};
use parking_lot::RwLock;
use std::{
    collections::HashSet,
    sync::{Arc, OnceLock},
};

pub trait MediaPlayerAddon: Send + Sync {
    /// Lowercase namespace, e.g. `example_radio`; `liasmediaplayer` is reserved.
    fn id(&self) -> &str;
    fn api_version(&self) -> u32 {
        super::API_VERSION
    }
    fn register(&self, context: &mut AddonContext) -> Result<(), RegisterError>;
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RegisterError(pub String);

/// Registration is transactional: invalid IDs, duplicates, errors and panics commit nothing.
#[derive(Default)]
pub struct AddonContext {
    sources: Vec<Arc<dyn LinkSource>>,
    resolvers: Vec<Arc<dyn Resolver>>,
    themes: Vec<ThemeSpec>,
    actions: Vec<WindowAction>,
}
impl AddonContext {
    pub fn source(&mut self, source: impl LinkSource + 'static) {
        self.sources.push(Arc::new(source));
    }
    pub fn resolver(&mut self, resolver: impl Resolver + 'static) {
        self.resolvers.push(Arc::new(resolver));
    }
    pub fn theme(&mut self, theme: ThemeSpec) {
        self.themes.push(theme);
    }
    pub fn window_action(&mut self, action: WindowAction) {
        self.actions.push(action);
    }
}
#[derive(Default)]
struct Registry {
    addons: HashSet<String>,
    sources: Vec<Arc<dyn LinkSource>>,
    resolvers: Vec<Arc<dyn Resolver>>,
    themes: Vec<ThemeSpec>,
    actions: Vec<WindowAction>,
}
fn registry() -> &'static RwLock<Registry> {
    static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}
fn valid(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.bytes()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}
fn validate_ids<'a>(owner: &str, ids: impl Iterator<Item = &'a str>) -> Result<(), RegisterError> {
    let mut seen = HashSet::new();
    for id in ids {
        if !id
            .split_once(':')
            .is_some_and(|(ns, name)| ns == owner && valid(name))
            || !seen.insert(id)
        {
            return Err(RegisterError(format!(
                "Invalid or duplicate addon ID: {id}"
            )));
        }
    }
    Ok(())
}
/// Safe from any thread, including a host hook that already holds the runtime lock.
/// Registration callbacks run outside registry locks. Late registration is supported;
/// register before opening screens to apply persisted addon themes at startup.
pub fn register_addon(addon: Box<dyn MediaPlayerAddon>) -> Result<(), RegisterError> {
    let id = addon.id().to_owned();
    if !valid(&id) || id == crate::ID {
        return Err(RegisterError("Invalid addon namespace".into()));
    }
    if addon.api_version() != super::API_VERSION {
        return Err(RegisterError("Unsupported addon API version".into()));
    }
    if registry().read().addons.contains(&id) {
        return Err(RegisterError(format!("Addon already registered: {id}")));
    }
    let mut context = AddonContext::default();
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        addon.register(&mut context)
    }))
    .map_err(|_| RegisterError(format!("Addon registration panicked: {id}")))??;
    validate_ids(&id, context.sources.iter().map(|s| s.id()))?;
    validate_ids(&id, context.resolvers.iter().map(|s| s.id()))?;
    validate_ids(&id, context.themes.iter().map(|s| s.id.as_str()))?;
    validate_ids(&id, context.actions.iter().map(|s| s.id.as_str()))?;
    if context
        .actions
        .iter()
        .any(|a| a.label.chars().count() != 1 || a.label.chars().any(char::is_control))
    {
        return Err(RegisterError(
            "Window action labels must be one printable character".into(),
        ));
    }
    // Evaluate addon-owned methods before taking the registry write lock.
    context
        .sources
        .sort_by_key(|s| std::cmp::Reverse(s.priority()));
    let mut r = registry().write();
    if r.addons.contains(&id) {
        return Err(RegisterError(format!("Addon already registered: {id}")));
    }
    if r.actions.len() + context.actions.len() > 4 {
        return Err(RegisterError(
            "At most four addon window actions are supported".into(),
        ));
    }
    r.sources.extend(context.sources);
    r.resolvers.extend(context.resolvers);
    r.themes.extend(context.themes);
    r.actions.extend(context.actions);
    r.addons.insert(id);
    Ok(())
}
pub(super) fn themes() -> Vec<ThemeSpec> {
    registry().read().themes.clone()
}
pub(super) fn window_actions() -> Vec<WindowAction> {
    registry().read().actions.clone()
}
pub(crate) fn sources() -> Vec<Arc<dyn LinkSource>> {
    registry().read().sources.clone()
}
pub(crate) fn resolvers() -> Vec<Arc<dyn Resolver>> {
    registry().read().resolvers.clone()
}
