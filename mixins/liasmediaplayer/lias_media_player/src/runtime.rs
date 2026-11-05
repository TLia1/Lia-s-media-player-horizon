//! Client runtime: settings, persistence, UI, asynchronous images and the status badge.

mod screens;

use crate::command::{ShowCommand, ShowKind};
use parking_lot::Mutex;
use std::sync::OnceLock;
use std::time::Duration;

/// Default background color for the HUD badge (`0x78000000`, matching Horizon's extras HUD box).
pub const BADGE_BG_ARGB: u32 = 0x7800_0000;
/// Accent bar color on the left edge of the HUD badge (`0xFF55FF55` emerald green).
pub const BADGE_ACCENT_ARGB: u32 = 0xFF55_FF55;
/// Primary text color (`0xFFFFFF` white).
pub const BADGE_TEXT_RGB: u32 = 0xFF_FFFF;
/// Secondary status text color (`0xAAAAAA` light gray).
pub const BADGE_SUBTEXT_RGB: u32 = 0xAA_AAAA;

/// Setting key stored via `Game::native_setting("liasmediaplayer", KEY_HUD_BADGE)`.
pub const KEY_HUD_BADGE: &str = "hud_badge";
/// Setting key stored via `Game::native_setting("liasmediaplayer", KEY_REPLACE_CHAT_LINKS)`.
pub const KEY_REPLACE_CHAT_LINKS: &str = "replace_chat_links";

/// Keybind definitions registered in `NativeMod.keys`: `[name, label, default_key]`.
pub const MOD_KEYS: &[[&str; 3]] = &[
    ["controls", "Open Media Controls", "key.keyboard.m"],
    ["play_pause", "Play / Pause", "key.keyboard.k"],
    ["next", "Next Track", "key.keyboard.n"],
    ["playlists", "Open Playlists", "key.keyboard.p"],
    ["history", "Open Media History", "key.keyboard.h"],
];

/// A two-line HUD badge rendered in `native_hud` during Milestone 0 to verify the native
/// mod is loaded, ticking, and drawing through `HudBatch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Badge {
    pub header: String,
    pub status: String,
    pub progress: Option<u16>,
}

/// Runtime owned by the client; the global instance is protected by a mutex.
pub struct Runtime {
    hud_badge_shown: Option<bool>,
    replace_chat_links: Option<bool>,
    ticks: u64,
    frames: u64,
    elapsed: Duration,
    last_command: Option<ShowCommand>,
    pub ui: crate::ui::demo::UiDemo,
    pub images: Option<crate::core::images::Images>,
    pub stores: Option<crate::core::store::Stores>,
    storage_path: Option<std::path::PathBuf>,
    restore_images: bool,
    pub playback: Option<crate::core::playback::PlaybackRegistry>,
    pub interactive: bool,
    disabled: bool,
    pub chat: crate::ui::chat_overlay::ChatOverlay,
    pub previews: Option<crate::core::previews::PreviewCache>,
    pub sources: crate::core::sources::SourceRegistry,
    pub chat_hover_previews: bool,
    pub open_playlist_screen: bool,
    pub screens: crate::ui::screens::Screens,
    pub close_screen_requested: bool,
    pub chat_rewrap_requested: bool,
    import_task: Option<(String, crate::core::previews::CollectionTask)>,
    tool_task: Option<crate::core::tools::ToolTask>,
    chat_tools: crate::core::tools::ToolManager,
    collection: Option<crate::core::previews::CollectionTask>,
    collection_show: Option<ShowCommand>,
    addon_observer: crate::api::media::Observer,
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            hud_badge_shown: None,
            replace_chat_links: None,
            ticks: 0,
            frames: 0,
            elapsed: Duration::ZERO,
            last_command: None,
            ui: crate::ui::demo::UiDemo::default(),
            images: None,
            stores: None,
            storage_path: None,
            restore_images: true,
            playback: None,
            interactive: false,
            disabled: false,
            chat: Default::default(),
            previews: None,
            sources: Default::default(),
            chat_hover_previews: true,
            open_playlist_screen: false,
            screens: Default::default(),
            close_screen_requested: false,
            chat_rewrap_requested: false,
            import_task: None,
            tool_task: None,
            chat_tools: crate::core::tools::ToolManager::new(None),
            collection: None,
            collection_show: None,
            addon_observer: Default::default(),
        }
    }

    /// Called once the host supplies its data paths, and again on world entry.
    pub fn configure(&mut self, root: &std::path::Path, java: &std::path::Path) {
        if self.storage_path.as_deref() != Some(root) {
            self.storage_path = Some(root.into());
            match crate::core::store::Stores::open(root, java) {
                Ok(stores) => {
                    let settings = stores.settings.get();
                    self.hud_badge_shown = settings["hudBadge"].as_bool();
                    self.replace_chat_links =
                        settings["replaceChatLinks"].as_bool().or_else(|| {
                            settings["values"]["liasmediaplayer:chat.replace_links"].as_bool()
                        });
                    self.chat_hover_previews = settings["chatHoverPreviews"]
                        .as_bool()
                        .or_else(|| {
                            settings["values"]["liasmediaplayer:chat.hover_previews"]
                                .as_bool()
                                .or_else(|| {
                                    settings["values"]["liasmediaplayer:chat.hover_preview"]
                                        .as_bool()
                                })
                        })
                        .unwrap_or(true);
                    let theme = settings["theme"]
                        .as_str()
                        .or_else(|| settings["values"]["liasmediaplayer:appearance.theme"].as_str())
                        .unwrap_or("dark");
                    let theme = theme.strip_prefix("liasmediaplayer:").unwrap_or(theme);
                    if let Some(found) = crate::api::themes()
                        .into_iter()
                        .find(|t| t.id.strip_prefix("liasmediaplayer:").unwrap_or(&t.id) == theme)
                    {
                        self.ui.windows.theme = found;
                    }
                    let memory = settings["memoryCacheSizeMb"]
                        .as_u64()
                        .or_else(|| {
                            settings["values"]["liasmediaplayer:performance.image_cache_mb"]
                                .as_u64()
                        })
                        .unwrap_or(128)
                        .clamp(16, 512) as usize
                        * 1024
                        * 1024;
                    let disk = settings["diskCacheSizeMb"]
                        .as_u64()
                        .or_else(|| {
                            settings["values"]["liasmediaplayer:performance.disk_cache_mb"].as_u64()
                        })
                        .unwrap_or(100)
                        .min(1000)
                        * 1024
                        * 1024;
                    let mut tools = crate::core::tools::ToolManager::new(Some(root));
                    tools.legacy_bin = Some(java.join("bin"));
                    tools.policy.prefer_system = settings["preferSystemBinaries"]
                        .as_bool()
                        .or_else(|| {
                            settings["values"]["liasmediaplayer:tools.prefer_system_binaries"]
                                .as_bool()
                        })
                        .unwrap_or(true);
                    tools.policy.auto_download = settings["autoDownloadTools"]
                        .as_bool()
                        .or_else(|| {
                            settings["values"]["liasmediaplayer:tools.auto_download"].as_bool()
                        })
                        .unwrap_or(true);
                    self.chat_tools = tools.clone();
                    self.previews = Some(crate::core::previews::PreviewCache::new(
                        Some(root),
                        tools.clone(),
                    ));
                    let mut playback = crate::core::playback::PlaybackRegistry::new(tools.clone());
                    playback.configure(settings);
                    self.playback = Some(playback);
                    self.images = Some(crate::core::images::Images::with_tools(
                        Some(root),
                        memory,
                        disk,
                        tools,
                    ));
                    self.stores = Some(stores);
                }
                Err(e) => {
                    log::warn!("Cannot open media stores: {e}");
                    self.images =
                        Some(crate::core::images::Images::new(None, 128 * 1024 * 1024, 0));
                }
            }
        }
        if let Some(playback) = &mut self.playback {
            playback.restore(&mut self.ui.windows);
        }
        self.disabled = false;
        if self.restore_images {
            self.restore_images = false;
            if let (Some(stores), Some(images)) = (&self.stores, &mut self.images)
                && let Some(pinned) = stores.windows.get()["pinned"].as_array()
            {
                for entry in pinned.iter().take(4) {
                    if let Some(url) = entry["url"].as_str() {
                        let _ = images.request(
                            url,
                            true,
                            &mut self.ui.windows,
                            &mut self.ui.atlas,
                            Some(entry),
                        );
                    }
                }
            }
        }
    }
    pub(crate) fn addon_available(&self) -> bool {
        self.storage_path.is_some() && !self.disabled && !self.restore_images
    }
    fn sample_addon_events(&mut self) {
        let mut observer = std::mem::take(&mut self.addon_observer);
        observer.sample(self);
        self.addon_observer = observer;
    }
    pub fn open_controls(&mut self) {
        self.ui.windows.reveal();
        self.open_screen(crate::ui::screens::ScreenKind::Controls);
    }
    pub fn input(&mut self, event: &crate::platform::InputEvent) -> bool {
        // Save the last geometry before a close can remove its window.
        if let (Some(images), Some(stores)) = (&self.images, &mut self.stores) {
            images.persist(&self.ui.windows, stores);
        }
        let screen_tab = self.screens.active.is_some()
            && !self.ui.windows.has_focus()
            && matches!(
                event,
                crate::platform::InputEvent::Key {
                    key: crate::platform::Key::Tab,
                    ..
                }
            );
        let windows_visible = self.screens.active.is_none()
            || self.screens.active == Some(crate::ui::screens::ScreenKind::Controls);
        let used = windows_visible && !screen_tab && self.ui.event(event);
        if used && matches!(event, crate::platform::InputEvent::Down { .. }) {
            self.screens.cancel_input();
        }
        let used = if !used && self.screens.active.is_some() && self.chat.picker.is_none() {
            if matches!(event, crate::platform::InputEvent::Key { key: crate::platform::Key::V, mods } if mods.ctrl)
            {
                match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                    Ok(text) => {
                        self.screens.paste(&text);
                    }
                    Err(e) => self.screens.notice = format!("Clipboard unavailable: {e}"),
                }
                self.sync_screens();
                return true;
            }
            let (used, actions) = self.screens.input(event);
            for action in actions {
                self.screen_action(action);
            }
            self.sync_screens();
            used
        } else if !used && self.chat.picker.is_some() {
            let (used, action) = self.chat.picker_input(event, &self.sources);
            if let Some(action) = action {
                self.chat_action(action);
            }
            used
        } else {
            used
        };
        if let Some(stores) = &mut self.stores {
            let mut settings = stores.settings.get().clone();
            settings["theme"] = self.ui.windows.theme.id.clone().into();
            if settings["values"].is_object() {
                settings["values"]["liasmediaplayer:appearance.theme"] =
                    self.ui.windows.theme.id.clone().into();
            }
            if let Err(e) = stores.settings.set(settings) {
                log::warn!("Theme settings: {e}");
            }
            if let Some(images) = &self.images {
                images.persist(&self.ui.windows, stores);
            }
        }
        used
    }

    /// Whether the HUD test badge is shown, initializing from `load_saved` on first read.
    pub fn hud_badge_shown(&mut self, load_saved: impl FnOnce() -> Option<String>) -> bool {
        *self.hud_badge_shown.get_or_insert_with(|| {
            load_saved()
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true)
        })
    }

    /// Sets whether the HUD test badge is shown.
    pub fn set_hud_badge_shown(&mut self, shown: bool) {
        self.hud_badge_shown = Some(shown);
        if let Some(stores) = &mut self.stores {
            let mut settings = stores.settings.get().clone();
            settings["hudBadge"] = shown.into();
            if let Err(e) = stores.settings.set(settings) {
                log::warn!("Settings: {e}");
            }
        }
        self.screens.invalidate();
    }

    /// Whether chat link replacement is enabled, initializing from `load_saved` on first read.
    pub fn replace_chat_links(&mut self, load_saved: impl FnOnce() -> Option<String>) -> bool {
        *self.replace_chat_links.get_or_insert_with(|| {
            load_saved()
                .map(|v| v != "false" && v != "0")
                .unwrap_or(true)
        })
    }

    /// Sets whether chat link replacement is enabled.
    pub fn set_replace_chat_links(&mut self, enabled: bool) {
        self.replace_chat_links = Some(enabled);
        self.screens.invalidate();
        if let Some(stores) = &mut self.stores {
            let mut settings = stores.settings.get().clone();
            settings["replaceChatLinks"] = enabled.into();
            if let Err(e) = stores.settings.set(settings) {
                log::warn!("Settings: {e}");
            }
        }
    }

    /// Button labels for `Options > Mods > Lia's Media Player`.
    pub fn settings_buttons(
        &mut self,
        load_badge: impl FnOnce() -> Option<String>,
        load_chat: impl FnOnce() -> Option<String>,
    ) -> Vec<String> {
        let badge = self.hud_badge_shown(load_badge);
        let chat = self.replace_chat_links(load_chat);
        vec![
            format!("HUD Status Badge: {}", if badge { "ON" } else { "OFF" }),
            format!("Replace Chat Links: {}", if chat { "ON" } else { "OFF" }),
            "Open Full Media Player Settings...".to_owned(),
        ]
    }

    /// Advance the client tick counter (`native_tick`).
    pub fn on_tick(&mut self) {
        self.ticks = self.ticks.saturating_add(1);
    }

    /// Advance the frame counter (`native_frame`).
    pub fn on_frame(&mut self, elapsed: Duration) {
        self.chat.tick(self.previews.as_mut(), &mut self.ui.atlas);
        self.poll_collection();
        self.poll_screen_jobs();
        self.frames = self.frames.saturating_add(1);
        self.elapsed = self.elapsed.saturating_add(elapsed);
        if let Some(playback) = &mut self.playback {
            playback.tick(
                elapsed,
                &mut self.ui.windows,
                &mut self.ui.atlas,
                self.stores.as_mut(),
                self.interactive,
            );
        }
        if !self.restore_images
            && let Some(images) = &mut self.images
        {
            images.tick(
                elapsed,
                &self.ui.windows,
                &mut self.ui.atlas,
                self.stores.as_mut(),
            );
            if let Some(stores) = &mut self.stores {
                images.persist(&self.ui.windows, stores);
            }
        }
        self.sync_screens();
        self.sample_addon_events();
    }

    /// Record an accepted `/show` command and return the chat confirmation message.
    pub fn execute_show(&mut self, cmd: ShowCommand) -> String {
        if matches!(cmd.kind, ShowKind::Audio | ShowKind::Video)
            && let Some(mut media) = self
                .sources
                .identify(&cmd.url)
                .filter(|m| m.kind == crate::core::sources::MediaKind::Collection)
        {
            if self.collection.is_some() {
                return "[Lia's Media Player] A playlist is already loading".into();
            }
            media.entry_kind = if cmd.kind == ShowKind::Audio {
                crate::core::sources::MediaKind::Audio
            } else {
                crate::core::sources::MediaKind::Video
            };
            self.collection = Some(crate::core::previews::CollectionTask::start(
                self.chat_tools.clone(),
                media,
                true,
            ));
            self.collection_show = Some(cmd.clone());
            self.last_command = Some(cmd);
            self.chat.notify("Loading playlist…");
            return "[Lia's Media Player] Loading playlist…".into();
        }
        if matches!(cmd.kind, ShowKind::Image | ShowKind::Gif) {
            self.restore_images = false;
            if self.images.is_none() {
                self.images = Some(crate::core::images::Images::new(None, 128 * 1024 * 1024, 0));
            }
            let geometry = self
                .stores
                .as_ref()
                .map(|s| s.windows.get()["kinds"]["image"].clone());
            let result = self.images.as_mut().unwrap().request(
                &cmd.url,
                cmd.new_player,
                &mut self.ui.windows,
                &mut self.ui.atlas,
                geometry.as_ref(),
            );
            return match result {
                Ok(_) => {
                    let reply = format!("[Lia's Media Player] Loading {}: {}", cmd.kind, cmd.url);
                    self.last_command = Some(cmd);
                    reply
                }
                Err(e) => format!("[Lia's Media Player] {e}"),
            };
        }
        let playback = self.playback.get_or_insert_with(|| {
            crate::core::playback::PlaybackRegistry::new(crate::core::tools::ToolManager::new(None))
        });
        match playback.request(
            &cmd.url,
            cmd.kind == ShowKind::Audio,
            cmd.new_player,
            &mut self.ui.windows,
            &mut self.ui.atlas,
        ) {
            Ok(_) => {
                let reply = format!("[Lia's Media Player] Queued {}: {}", cmd.kind, cmd.url);
                self.last_command = Some(cmd);
                reply
            }
            Err(error) => format!("[Lia's Media Player] {error}"),
        }
    }

    pub fn clear_chat(&mut self) {
        self.chat
            .clear_hover(self.previews.as_mut(), &mut self.ui.atlas);
        self.chat.picker = None;
        self.collection = None;
        self.collection_show = None;
        self.open_playlist_screen = false;
    }
    pub fn chat_action(&mut self, action: crate::core::chat::ChatAction) {
        use crate::core::chat::ChatAction;
        match action {
            ChatAction::RevealHidden => self.ui.windows.reveal(),
            ChatAction::Playlists => {
                self.open_screen(crate::ui::screens::ScreenKind::Playlists);
                self.open_playlist_screen = true;
            }
            ChatAction::Browser(url) => {
                // Only validated media URLs reach this action, including callers outside the bridge.
                if crate::core::sources::web_url(&url).is_some() {
                    std::thread::spawn(move || {
                        if let Err(e) = open::that(&url) {
                            log::warn!("Browser: {e}");
                        }
                    });
                }
            }
            ChatAction::Open(media) => self.open_chat_media(media, false),
            ChatAction::Enqueue(media) => self.open_chat_media(media, true),
        }
    }
    fn open_chat_media(&mut self, media: crate::core::sources::MediaRef, enqueue: bool) {
        use crate::core::sources::MediaKind;
        if media.kind == MediaKind::Collection {
            if self.collection.is_some() {
                self.chat.notify("A playlist is already loading");
                return;
            }
            self.collection_show = None;
            self.collection = Some(crate::core::previews::CollectionTask::start(
                self.chat_tools.clone(),
                media,
                true,
            ));
            self.chat.notify("Loading playlist as audio…");
            return;
        }
        if enqueue && media.kind == MediaKind::Image {
            self.chat
                .notify("Images have no playback queue. Click to open.");
            return;
        }
        let reply = self.execute_show(ShowCommand {
            kind: match media.kind {
                MediaKind::Image => ShowKind::Image,
                MediaKind::Audio => ShowKind::Audio,
                _ => ShowKind::Video,
            },
            url: media.url,
            new_player: !enqueue,
        });
        self.chat.notify(reply);
    }
    fn poll_collection(&mut self) {
        let Some(task) = &self.collection else {
            return;
        };
        let Some(result) = task.poll() else {
            return;
        };
        let enqueue = task.enqueue;
        let show = self.collection_show.take();
        self.collection = None;
        match result {
            Err(e) => {
                let notice = format!("Collection: {e}");
                if self.screens.active == Some(crate::ui::screens::ScreenKind::Controls) {
                    self.screens.notice = notice.clone();
                }
                self.chat.notify(notice);
            }
            Ok(collection) if enqueue => {
                let audio = show.as_ref().is_none_or(|cmd| cmd.kind == ShowKind::Audio);
                let titles = collection
                    .entries
                    .iter()
                    .map(|m| (m.url.clone(), m.label.clone()))
                    .collect();
                let playback = self.playback.get_or_insert_with(|| {
                    crate::core::playback::PlaybackRegistry::new(self.chat_tools.clone())
                });
                let result = playback.request_collection(
                    collection.entries.into_iter().map(|m| m.url).collect(),
                    audio,
                    show.as_ref().is_some_and(|cmd| cmd.new_player),
                    &mut self.ui.windows,
                    &mut self.ui.atlas,
                );
                playback.remember_titles(titles);
                let notice = match result {
                    Ok(()) => {
                        if audio {
                            "Playlist queued as audio".into()
                        } else {
                            "Playlist queued as video".into()
                        }
                    }
                    Err(e) => e,
                };
                if self.screens.active == Some(crate::ui::screens::ScreenKind::Controls) {
                    self.screens.notice = notice.clone();
                }
                self.chat.notify(notice);
            }
            Ok(collection) => {
                self.chat.show_picker(
                    collection.title.unwrap_or("Collection".into()),
                    collection
                        .entries
                        .into_iter()
                        .map(|media| crate::ui::chat_overlay::PickerRow {
                            label: media.label.clone(),
                            media: Some(media),
                        })
                        .collect(),
                );
                self.open_playlist_screen = true;
            }
        }
    }
    pub fn on_disabled(&mut self) {
        self.clear_chat();
        self.close_views();
        self.ui.windows.cancel_input();
        if !self.disabled {
            if let Some(playback) = &mut self.playback {
                playback.suspend(&mut self.ui.atlas);
            }
            self.disabled = true;
        }
    }

    /// Clear transient state when a world unloads (`native_client_level_unload`).
    pub fn on_world_unload(&mut self) {
        self.clear_chat();
        self.close_views();
        if let Some(playback) = &mut self.playback {
            playback.suspend(&mut self.ui.atlas);
        }
        if let (Some(images), Some(stores)) = (&self.images, &mut self.stores) {
            images.persist(&self.ui.windows, stores);
        }
        if let Some(stores) = &self.stores
            && let Err(e) = stores.flush()
        {
            log::warn!("Save on world unload: {e}");
        }
        if let Some(images) = &mut self.images {
            images.clear(&mut self.ui.atlas);
        }
        self.last_command = None;
        self.ui.windows.clear();
        self.ui.atlas = crate::platform::horizon_impl::ModAtlasPacker::new();
        self.restore_images = true;
        self.sample_addon_events();
    }

    /// Most recent `/show` command accepted in this session.
    pub fn last_command(&self) -> Option<&ShowCommand> {
        self.last_command.as_ref()
    }

    /// Only show the HUD while there is media or a pending collection.
    pub fn hud_active(&self) -> bool {
        self.collection.is_some() || !self.ui.windows.windows().is_empty()
    }
    pub fn badge(&self) -> Badge {
        if self.collection.is_some() {
            return Badge {
                header: "Loading playlist…".into(),
                status: "Fetching tracks from yt-dlp".into(),
                progress: None,
            };
        }
        if let Some(playback) = &self.playback
            && let Some((window, player)) = self
                .ui
                .windows
                .windows()
                .iter()
                .rev()
                .find_map(|w| playback.get(w.id).map(|p| (w, p)))
        {
            let snapshot = player.player.control.snapshot();
            let title = if snapshot.title.is_empty() {
                "Loading media…"
            } else {
                &snapshot.title
            };
            let clock = |s: f64| {
                let s = s.max(0.) as u64;
                format!("{}:{:02}", s / 60, s % 60)
            };
            let time = if snapshot.is_live {
                "LIVE".into()
            } else {
                format!(
                    "{} / {}",
                    clock(snapshot.position_secs),
                    snapshot.duration_secs.map(clock).unwrap_or("--:--".into())
                )
            };
            let status = if snapshot.state == crate::core::player::PlaybackState::Failed {
                "Playback failed · Open Controls to retry".into()
            } else {
                format!(
                    "{time} · {} queued · {}%{}",
                    snapshot.queue.len(),
                    (snapshot.volume * 100.).round(),
                    if window.hidden { " · hidden" } else { "" }
                )
            };
            return Badge {
                header: format!("{:?} · {title}", snapshot.state),
                status,
                progress: snapshot
                    .duration_secs
                    .filter(|d| *d > 0. && !snapshot.is_live)
                    .map(|d| ((snapshot.position_secs / d).clamp(0., 1.) * 1000.) as u16),
            };
        }
        Badge {
            header: "Lia's Media Player".into(),
            status: if self.ui.windows.windows().is_empty() {
                "Ready".into()
            } else {
                format!("{} image window(s)", self.ui.windows.windows().len())
            },
            progress: None,
        }
    }

    pub fn ticks(&self) -> u64 {
        self.ticks
    }

    pub fn frames(&self) -> u64 {
        self.frames
    }
}

/// Access the process-wide `Runtime` singleton.
fn runtime_mutex() -> &'static Mutex<Runtime> {
    static RUNTIME: OnceLock<Mutex<Runtime>> = OnceLock::new();
    RUNTIME.get_or_init(|| Mutex::new(Runtime::new()))
}
/// Nonblocking public API access: never deadlock when called from a host callback.
pub(crate) fn try_with_runtime<R>(f: impl FnOnce(&mut Runtime) -> R) -> Result<R, String> {
    let mut guard = runtime_mutex()
        .try_lock()
        .ok_or("Media runtime is busy; retry later")?;
    Ok(f(&mut guard))
}
pub fn with_runtime<R>(f: impl FnOnce(&mut Runtime) -> R) -> R {
    let mutex = runtime_mutex();
    let mut guard = mutex.lock();
    f(&mut guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn badge_defaults_to_ready_and_updates_after_show() {
        let mut rt = Runtime::new();
        assert!(rt.hud_badge_shown(|| None));
        assert_eq!(rt.badge().status, "Ready");

        let reply = rt.execute_show(ShowCommand {
            kind: ShowKind::Video,
            url: "https://youtu.be/dQw4w9WgXcQ".into(),
            new_player: false,
        });
        assert!(reply.contains("Queued video: https://youtu.be/dQw4w9WgXcQ"));
        assert!(rt.hud_active());
        assert!(rt.badge().status.contains("queued"));

        rt.on_world_unload();
        assert!(rt.last_command().is_none());
    }

    #[test]
    fn settings_buttons_reflect_saved_values() {
        let mut rt = Runtime::new();
        let buttons = rt.settings_buttons(|| Some("false".into()), || Some("true".into()));
        assert_eq!(
            buttons,
            vec![
                "HUD Status Badge: OFF".to_owned(),
                "Replace Chat Links: ON".to_owned(),
                "Open Full Media Player Settings...".to_owned(),
            ]
        );
    }

    #[test]
    fn tick_and_frame_counters_advance() {
        let mut rt = Runtime::new();
        rt.on_tick();
        rt.on_tick();
        rt.on_frame(Duration::from_millis(16));
        assert_eq!(rt.ticks(), 2);
        assert_eq!(rt.frames(), 1);
    }
}
