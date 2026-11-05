// No dependency on the media crate in the renderer. The game installs a display callback.
use std::sync::{OnceLock, atomic::{AtomicBool, Ordering}};
type Transform = fn(crate::text::Text) -> crate::text::Text;
static TRANSFORM: OnceLock<Transform> = OnceLock::new();
static ENABLED: AtomicBool = AtomicBool::new(false);
pub fn set(enabled: bool, transform: Transform) -> bool {
    let _ = TRANSFORM.set(transform);
    ENABLED.swap(enabled, Ordering::Relaxed) != enabled
}
pub fn transform(message: crate::text::Text) -> crate::text::Text {
    if ENABLED.load(Ordering::Relaxed) && let Some(transform) = TRANSFORM.get() { transform(message) } else { message }
}
