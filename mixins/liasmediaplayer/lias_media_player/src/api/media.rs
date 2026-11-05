use super::{MediaKind, PlaybackState, PlayerCommand, PlayerSnapshot};
use crate::runtime::Runtime;
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    sync::{
        Arc, OnceLock, Weak,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
};

/// Session-local identity. IDs are never reused; closed handles return an error.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MediaHandle(pub(crate) u64);
impl MediaHandle {
    pub fn id(self) -> u64 {
        self.0
    }
    pub fn snapshot(self) -> Result<MediaSnapshot, String> {
        crate::runtime::try_with_runtime(|r| {
            snapshot(r, self).ok_or_else(|| "Media window is closed".into())
        })?
    }
    /// Enqueues actor work. Success means admitted, not completed; observe snapshots/events.
    /// Image controls support Play/Pause/Toggle only; use window commands for geometry.
    pub fn command(self, command: PlayerCommand) -> Result<(), String> {
        crate::runtime::try_with_runtime(|r| {
            if !r.addon_available() {
                return Err("Media player is not active in a world".into());
            }
            if !r.ui.windows.windows().iter().any(|w| w.id == self.0) {
                return Err("Media window is closed".into());
            }
            if let Some(p) = r.playback.as_ref().and_then(|p| p.get(self.0)) {
                return p.player.control.send(command);
            }
            let view = r
                .images
                .as_ref()
                .and_then(|i| i.view(self.0))
                .ok_or("Media window is closed")?;
            let mut view = view.lock();
            match command {
                PlayerCommand::Play => view.paused = false,
                PlayerCommand::Pause => view.paused = true,
                PlayerCommand::Toggle => view.paused = !view.paused,
                _ => return Err("Unsupported image command".into()),
            }
            Ok(())
        })?
    }
    pub fn window(self, command: WindowCommand) -> Result<(), String> {
        crate::runtime::try_with_runtime(|r| {
            let w =
                r.ui.windows
                    .window_mut(self.0)
                    .ok_or("Media window is closed")?;
            match command {
                WindowCommand::Show => w.hidden = false,
                WindowCommand::Hide => w.hidden = true,
                WindowCommand::Pin(value) => w.pinned = value,
                WindowCommand::Theatre(value) => w.theatre = value,
                WindowCommand::Close => {
                    if let Some(p) = r.playback.as_ref().and_then(|p| p.get(self.0)) {
                        p.player.control.close();
                    }
                    r.ui.windows.close(self.0);
                }
            }
            Ok(())
        })?
    }
}
#[derive(Clone, Copy, Debug)]
pub enum WindowCommand {
    Show,
    Hide,
    Pin(bool),
    Theatre(bool),
    Close,
}
#[derive(Clone, Debug)]
pub struct MediaSnapshot {
    pub handle: MediaHandle,
    pub kind: MediaKind,
    pub source_url: String,
    pub title: String,
    pub state: PlaybackState,
    pub hidden: bool,
    pub pinned: bool,
    pub error: Option<String>,
    /// Audio/video transport details; images have no transport clock.
    pub playback: Option<Arc<PlayerSnapshot>>,
}
#[derive(Clone, Debug)]
pub enum MediaEvent {
    Opened(MediaSnapshot),
    /// Sampled once per host frame; position-only changes do not emit events.
    Changed(MediaSnapshot),
    Closed(MediaHandle),
    WindowAction {
        id: String,
        handle: MediaHandle,
    },
}
struct Subscriber {
    alive: Weak<()>,
    sender: SyncSender<MediaEvent>,
}
/// Event receiver; dereferences to the standard channel receiver for `try_recv`,
/// `recv_timeout`, and iteration. Dropping it immediately releases admission capacity.
pub struct EventSubscription {
    receiver: Receiver<MediaEvent>,
    _alive: Arc<()>,
}
impl std::ops::Deref for EventSubscription {
    type Target = Receiver<MediaEvent>;
    fn deref(&self) -> &Self::Target {
        &self.receiver
    }
}
fn subscribers() -> &'static Mutex<Vec<Subscriber>> {
    static SUBSCRIBERS: OnceLock<Mutex<Vec<Subscriber>>> = OnceLock::new();
    SUBSCRIBERS.get_or_init(Default::default)
}
/// A bounded session event stream (1–1024 entries). Drop the receiver to unsubscribe.
/// If a consumer falls behind it is disconnected rather than blocking the game;
/// drain buffered events, resubscribe, and call `active_media()` to resynchronize.
/// A maximum of 64 subscriptions is allowed.
pub fn subscribe(capacity: usize) -> Result<EventSubscription, String> {
    let mut subscribers = subscribers().lock();
    subscribers.retain(|s| s.alive.strong_count() > 0);
    if subscribers.len() >= 64 {
        return Err("Too many media event subscriptions".into());
    }
    let (tx, rx) = mpsc::sync_channel(capacity.clamp(1, 1024));
    let alive = Arc::new(());
    subscribers.push(Subscriber {
        alive: Arc::downgrade(&alive),
        sender: tx,
    });
    Ok(EventSubscription {
        receiver: rx,
        _alive: alive,
    })
}
pub(crate) fn publish(event: MediaEvent) {
    subscribers()
        .lock()
        .retain(|tx| match tx.sender.try_send(event.clone()) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        });
}
/// Opens a separate window using the same limits and tool policy as the UI.
/// Collection expansion is separate; only Image, Audio and Video are accepted here.
pub fn open(url: &str, kind: MediaKind) -> Result<MediaHandle, String> {
    crate::core::sources::web_url(url).ok_or("Expected an HTTP(S) URL")?;
    crate::runtime::try_with_runtime(|r| {
        if !r.addon_available() {
            return Err("Media player is not active in a world".into());
        }
        let id = match kind {
            MediaKind::Image => r
                .images
                .as_mut()
                .ok_or("Image pipeline unavailable")?
                .request(url, true, &mut r.ui.windows, &mut r.ui.atlas, None)?,
            MediaKind::Audio | MediaKind::Video => {
                r.playback.as_mut().ok_or("Playback unavailable")?.request(
                    url,
                    kind == MediaKind::Audio,
                    true,
                    &mut r.ui.windows,
                    &mut r.ui.atlas,
                )?
            }
            MediaKind::Collection => return Err("Expand collections before opening media".into()),
        };
        Ok(MediaHandle(id))
    })?
}
pub fn active_media() -> Result<Vec<MediaSnapshot>, String> {
    crate::runtime::try_with_runtime(|r| snapshots(r).into_values().collect())
}
fn snapshot(r: &Runtime, handle: MediaHandle) -> Option<MediaSnapshot> {
    let w = r.ui.windows.windows().iter().find(|w| w.id == handle.0)?;
    let (kind, url, title, state, error, playback) =
        if let Some(p) = r.playback.as_ref().and_then(|p| p.get(w.id)) {
            let s = p.player.control.snapshot();
            (
                if p.audio_only {
                    MediaKind::Audio
                } else {
                    MediaKind::Video
                },
                s.source_url.clone(),
                s.title.clone(),
                s.state,
                s.error.clone(),
                Some(s),
            )
        } else {
            let images = r.images.as_ref()?;
            let view = images.view(w.id)?;
            let view = view.lock();
            let state = if view.image.is_some() {
                if view.paused {
                    PlaybackState::Paused
                } else {
                    PlaybackState::Playing
                }
            } else if view.status == "Loading image…" {
                PlaybackState::Buffering
            } else {
                PlaybackState::Failed
            };
            (
                MediaKind::Image,
                images.source_url(w.id)?.to_owned(),
                w.title.clone(),
                state,
                (state == PlaybackState::Failed).then(|| view.status.clone()),
                None,
            )
        };
    Some(MediaSnapshot {
        handle,
        kind,
        source_url: url,
        title,
        state,
        hidden: w.hidden,
        pinned: w.pinned,
        error,
        playback,
    })
}
fn snapshots(r: &Runtime) -> BTreeMap<MediaHandle, MediaSnapshot> {
    r.ui.windows
        .windows()
        .iter()
        .filter_map(|w| snapshot(r, MediaHandle(w.id)).map(|s| (s.handle, s)))
        .collect()
}
#[derive(Default)]
pub(crate) struct Observer(BTreeMap<MediaHandle, MediaSnapshot>);
impl Observer {
    pub(crate) fn sample(&mut self, r: &Runtime) {
        if !subscribers()
            .lock()
            .iter()
            .any(|s| s.alive.strong_count() > 0)
        {
            self.0.clear();
            return;
        }
        let current = snapshots(r);
        for (id, s) in &current {
            match self.0.get(id) {
                None => publish(MediaEvent::Opened(s.clone())),
                Some(old)
                    if old.state != s.state
                        || old.source_url != s.source_url
                        || old.title != s.title
                        || old.hidden != s.hidden
                        || old.pinned != s.pinned
                        || old.error != s.error =>
                {
                    publish(MediaEvent::Changed(s.clone()))
                }
                _ => (),
            }
        }
        for id in self.0.keys().filter(|id| !current.contains_key(id)) {
            publish(MediaEvent::Closed(*id));
        }
        self.0 = current;
    }
}
