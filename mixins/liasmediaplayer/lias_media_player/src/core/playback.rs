//! Window ownership, admission limits, resize debounce and hidden-player policy.
use super::{
    audio::{AudioFactory, CpalAudioSink},
    player::{PlaybackState, Player, PlayerCommand},
    tools::ToolManager,
};
use crate::{
    platform::{Canvas, Rect, horizon_impl::ModAtlasPacker},
    ui::{
        FloatingWindow, WindowManager,
        video_window::{AudioBar, NowPlayingBanner, PlaybackView, VideoWindow},
    },
};
use parking_lot::Mutex;
use std::{collections::HashMap, sync::Arc, time::Duration};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackgroundMode {
    KeepVideo,
    SoundOnlyAfter5s,
}
pub struct Playback {
    pub player: Player,
    pub view: Arc<Mutex<PlaybackView>>,
    pub audio_only: bool,
    geometry: Rect,
    pinned: bool,
    texture_size: Option<(u32, u32)>,
    hidden_for: Duration,
    requested_video: bool,
    resize: (u32, Duration),
    generation: u64,
    history_url: String,
    queue_expanded: bool,
}
pub struct PlaybackRegistry {
    entries: HashMap<u64, Playback>,
    tools: ToolManager,
    factory: AudioFactory,
    pub max_concurrent_videos: usize,
    pub background_mode: BackgroundMode,
    suspended: bool,
    default_height: u32,
    default_fps: u32,
    default_volume: f32,
}
impl PlaybackRegistry {
    pub fn new(tools: ToolManager) -> Self {
        Self::with_audio_factory(tools, CpalAudioSink::factory())
    }
    pub fn with_audio_factory(tools: ToolManager, factory: AudioFactory) -> Self {
        Self {
            entries: HashMap::new(),
            tools,
            factory,
            max_concurrent_videos: 2,
            background_mode: BackgroundMode::SoundOnlyAfter5s,
            suspended: false,
            default_height: 720,
            default_fps: 30,
            default_volume: 1.,
        }
    }
    pub fn configure(&mut self, settings: &serde_json::Value) {
        self.max_concurrent_videos = settings["maxConcurrentVideos"]
            .as_u64()
            .or_else(|| {
                settings["values"]["liasmediaplayer:performance.max_concurrent_videos"].as_u64()
            })
            .unwrap_or(2)
            .clamp(1, 4) as usize;
        self.background_mode = if settings["backgroundMode"]
            .as_str()
            .or_else(|| settings["values"]["liasmediaplayer:playback.background_mode"].as_str())
            == Some("keep_video")
        {
            BackgroundMode::KeepVideo
        } else {
            BackgroundMode::SoundOnlyAfter5s
        };
        self.default_height = settings["defaultQuality"]
            .as_u64()
            .filter(|v| [360, 480, 720, 1080].contains(v))
            .unwrap_or(720) as u32;
        self.default_fps = if settings["videoFps"].as_u64() == Some(60) {
            60
        } else {
            30
        };
        self.default_volume = settings["masterVolume"]
            .as_f64()
            .filter(|v| v.is_finite())
            .unwrap_or(1.)
            .clamp(0., 1.) as f32;
    }
    pub fn set_tools(&mut self, tools: ToolManager) {
        self.tools = tools;
    }
    pub fn set_volume(&self, volume: f32) -> Result<(), String> {
        let mut result = Ok(());
        for p in self.entries.values() {
            if let Err(e) = p.player.control.send(PlayerCommand::SetVolume(volume)) {
                result = Err(e);
            }
        }
        result
    }
    pub fn get(&self, id: u64) -> Option<&Playback> {
        self.entries.get(&id)
    }
    pub fn request(
        &mut self,
        url: &str,
        audio_only: bool,
        new_player: bool,
        windows: &mut WindowManager,
        atlas: &mut ModAtlasPacker,
    ) -> Result<u64, String> {
        super::sources::web_url(url).ok_or("Expected an HTTP(S) URL")?;
        self.prune(windows, atlas);
        if !new_player
            && let Some(id) = windows
                .windows()
                .iter()
                .rev()
                .find(|w| {
                    self.entries
                        .get(&w.id)
                        .is_some_and(|p| p.audio_only == audio_only)
                })
                .map(|w| w.id)
        {
            self.entries[&id]
                .player
                .control
                .send(PlayerCommand::Enqueue(url.into()))?;
            windows.reveal();
            return Ok(id);
        }
        if !audio_only
            && self.entries.values().filter(|p| !p.audio_only).count() >= self.max_concurrent_videos
        {
            return Err(format!(
                "Close a video window first (maximum {})",
                self.max_concurrent_videos
            ));
        }
        if self.entries.len() >= 8 {
            return Err("Close a player first (maximum eight)".into());
        }
        let player =
            Player::with_audio_factory(self.tools.clone(), audio_only, self.factory.clone());
        let view = Arc::new(Mutex::new(PlaybackView::new(player.control.clone())));
        let content: Box<dyn crate::ui::Widget> = if audio_only {
            Box::new(AudioBar::window(view.clone()))
        } else {
            Box::new(VideoWindow::new(view.clone()))
        };
        let geometry = Rect::new(
            35. + self.entries.len() as f32 * 18.,
            40. + self.entries.len() as f32 * 18.,
            if audio_only { 270. } else { 258. },
            if audio_only { 31. } else { 175. },
        );
        let mut window = FloatingWindow::new(
            if audio_only { "Audio" } else { "Video" },
            geometry,
            content,
        );
        window.compact = true;
        window.min_size = [
            if audio_only { 200. } else { 160. },
            if audio_only { 31. } else { 71. },
        ];
        let id = window.id;
        player
            .control
            .send(PlayerCommand::SetTargetHeight(self.default_height))?;
        player
            .control
            .send(PlayerCommand::SetFps(self.default_fps))?;
        player
            .control
            .send(PlayerCommand::SetVolume(self.default_volume))?;
        player.control.send(PlayerCommand::Load(url.into()))?;
        windows.open(window);
        self.entries.insert(
            id,
            Playback {
                player,
                view,
                audio_only,
                geometry,
                pinned: true,
                texture_size: None,
                hidden_for: Duration::ZERO,
                requested_video: true,
                resize: (self.default_height, Duration::ZERO),
                generation: 0,
                history_url: String::new(),
                queue_expanded: false,
            },
        );
        self.suspended = false;
        Ok(id)
    }
    pub fn enqueue_collection(
        &mut self,
        urls: Vec<String>,
        audio: bool,
        windows: &mut WindowManager,
        atlas: &mut ModAtlasPacker,
    ) -> Result<(), String> {
        self.request_collection(urls, audio, false, windows, atlas)
    }
    pub fn request_collection(
        &mut self,
        urls: Vec<String>,
        audio: bool,
        new_player: bool,
        windows: &mut WindowManager,
        atlas: &mut ModAtlasPacker,
    ) -> Result<(), String> {
        if urls.is_empty() || urls.len() > 500 {
            return Err("Expected 1–500 collection entries".into());
        }
        self.prune(windows, atlas);
        if !new_player
            && let Some(p) = windows
                .windows()
                .iter()
                .rev()
                .filter_map(|w| self.entries.get(&w.id))
                .find(|p| p.audio_only == audio)
        {
            p.player.control.send(PlayerCommand::EnqueueBatch(urls))?;
            windows.reveal();
        } else {
            let id = self.request(&urls[0], audio, true, windows, atlas)?;
            self.entries[&id]
                .player
                .control
                .send(PlayerCommand::EnqueueBatch(
                    urls.into_iter().skip(1).collect(),
                ))?;
        }
        Ok(())
    }
    /// Saved playlists deliberately use an audio bar, matching the Java player.
    pub fn play_playlist(
        &mut self,
        mut entries: Vec<(String, String)>,
        shuffle: bool,
        repeat_all: bool,
        windows: &mut WindowManager,
        atlas: &mut ModAtlasPacker,
    ) -> Result<u64, String> {
        if entries.is_empty() || entries.len() > 500 {
            return Err("Choose a playlist with 1–500 tracks".into());
        }
        if shuffle {
            super::player::shuffle_slice(&mut entries);
        }
        let id = self.request(&entries[0].0, true, true, windows, atlas)?;
        let p = &self.entries[&id].player.control;
        p.send(PlayerCommand::SetTitles(entries.clone()))?;
        p.send(PlayerCommand::EnqueueBatch(
            entries.into_iter().skip(1).map(|(url, _)| url).collect(),
        ))?;
        p.send(PlayerCommand::SetShuffle(shuffle))?;
        p.send(PlayerCommand::SetRepeatAll(repeat_all))?;
        Ok(id)
    }
    pub fn remember_titles(&self, entries: Vec<(String, String)>) {
        for p in self.entries.values() {
            let _ = p
                .player
                .control
                .send(PlayerCommand::SetTitles(entries.clone()));
        }
    }
    fn prune(&mut self, windows: &WindowManager, atlas: &mut ModAtlasPacker) {
        self.entries.retain(|id, p| {
            if windows.windows().iter().any(|w| w.id == *id) {
                true
            } else {
                if let Some(t) = p.view.lock().texture.take() {
                    atlas.free(t);
                }
                false
            }
        });
    }
    pub fn tick(
        &mut self,
        dt: Duration,
        windows: &mut WindowManager,
        atlas: &mut ModAtlasPacker,
        mut stores: Option<&mut super::store::Stores>,
        interactive: bool,
    ) {
        if self.suspended {
            return;
        }
        self.prune(windows, atlas);
        for (&id, p) in &mut self.entries {
            let Some(window) = windows.window_mut(id) else {
                continue;
            };
            p.geometry = window.rect;
            p.pinned = window.pinned;
            let snapshot = p.player.control.snapshot();
            if !snapshot.title.is_empty() {
                window.title = snapshot.title.clone();
            }
            let mut view = p.view.lock();
            use crate::ui::video_window::{AUDIO_HEIGHT, DETAIL_HEIGHT, QUEUE_HEIGHT, QUEUE_WIDTH};
            if p.audio_only {
                let extra = if interactive && view.queue_open {
                    QUEUE_HEIGHT
                } else if interactive && view.details_open {
                    if snapshot.error.is_some() || view.error.is_some() {
                        DETAIL_HEIGHT
                    } else {
                        42.
                    }
                } else {
                    0.
                };
                window.rect.h = AUDIO_HEIGHT + extra;
                window.min_size = [200., window.rect.h];
                if snapshot.error.is_some() {
                    window.title = format!("Playback failed · {}", snapshot.title);
                }
            } else {
                let expanded = interactive && view.queue_open;
                if expanded != p.queue_expanded {
                    window.rect.w = (window.rect.w
                        + if expanded { QUEUE_WIDTH } else { -QUEUE_WIDTH })
                    .max(160.);
                    p.queue_expanded = expanded;
                }
                window.min_size = [
                    160. + if expanded { QUEUE_WIDTH } else { 0. },
                    if interactive && view.details_open {
                        AUDIO_HEIGHT + DETAIL_HEIGHT
                    } else {
                        71.
                    },
                ];
                window.rect.h = window.rect.h.max(window.min_size[1]);
            }
            if snapshot.generation != p.generation {
                p.generation = snapshot.generation;
                if let Some(t) = view.texture.take() {
                    atlas.free(t);
                }
                p.texture_size = None;
                view.size = None;
            }
            if let Some(frame) = p.player.control.take_frame() {
                let size = (frame.width, frame.height);
                if p.texture_size != Some(size) {
                    if let Some(t) = view.texture.take() {
                        atlas.free(t);
                    }
                    match atlas.allocate(frame.width, frame.height, frame.pixels()) {
                        Ok(t) => {
                            view.texture = Some(t);
                            p.texture_size = Some(size);
                            view.size = Some(size);
                            view.error = None;
                        }
                        Err(e) => {
                            view.error = Some(format!(
                                "Atlas full ({e:?}). Close another image/video window."
                            ));
                        }
                    }
                } else if let Some(t) = view.texture
                    && let Err(e) = atlas.update(t, frame.pixels())
                {
                    view.error = Some(format!("Video texture update failed: {e:?}"));
                }
            }
            if snapshot.state == PlaybackState::Playing && p.history_url != snapshot.source_url {
                p.history_url = snapshot.source_url.clone();
                if let Some(stores) = stores.as_deref_mut()
                    && let Err(e) = stores.record_media(
                        &snapshot.source_url,
                        if p.audio_only { "audio" } else { "video" },
                        &snapshot.title,
                    )
                {
                    log::warn!("Playback history: {e}");
                }
            }
            if !p.audio_only
                && matches!(
                    snapshot.state,
                    PlaybackState::Playing | PlaybackState::Paused
                )
            {
                let hidden = window.hidden || !interactive && !window.pinned;
                p.hidden_for = if hidden {
                    p.hidden_for.saturating_add(dt)
                } else {
                    Duration::ZERO
                };
                let video = self.background_mode == BackgroundMode::KeepVideo
                    || p.hidden_for < Duration::from_secs(5);
                if video != p.requested_video
                    && p.player
                        .control
                        .send(PlayerCommand::SetVideo(video))
                        .is_ok()
                {
                    p.requested_video = video;
                }
                if view.auto_quality && !hidden {
                    let height = if window.theatre {
                        1080
                    } else {
                        quality_for_height(window.rect.h)
                    };
                    if p.resize.0 != height {
                        p.resize = (height, Duration::ZERO);
                    } else {
                        p.resize.1 += dt;
                    }
                    if p.resize.1 >= Duration::from_millis(500) && snapshot.target_height != height
                    {
                        let _ = p
                            .player
                            .control
                            .send(PlayerCommand::SetTargetHeight(height));
                        p.resize.1 = Duration::ZERO;
                    }
                }
            }
        }
    }
    pub fn suspend(&mut self, atlas: &mut ModAtlasPacker) {
        self.suspended = true;
        for p in self.entries.values_mut() {
            let _ = p.player.control.send(PlayerCommand::Suspend);
            if let Some(t) = p.view.lock().texture.take() {
                atlas.free(t);
            }
            p.texture_size = None;
        }
    }
    pub fn restore(&mut self, windows: &mut WindowManager) {
        if !self.suspended {
            return;
        }
        self.suspended = false;
        for (&id, p) in &self.entries {
            let content: Box<dyn crate::ui::Widget> = if p.audio_only {
                Box::new(AudioBar::window(p.view.clone()))
            } else {
                Box::new(VideoWindow::new(p.view.clone()))
            };
            let mut window = FloatingWindow::new(
                p.player.control.snapshot().title.clone(),
                p.geometry,
                content,
            );
            window.id = id;
            window.pinned = p.pinned;
            window.compact = true;
            window.min_size = [
                if p.audio_only { 200. } else { 160. },
                if p.audio_only { 31. } else { 71. },
            ];
            windows.open(window);
        }
    }
    pub fn command_front(&self, windows: &WindowManager, command: PlayerCommand) {
        if let Some(p) = windows
            .windows()
            .iter()
            .rev()
            .find_map(|w| self.entries.get(&w.id))
        {
            let _ = p.player.control.send(command);
        }
    }
    pub fn draw_banner(&self, canvas: &mut dyn Canvas) {
        if !self.suspended
            && let Some(snapshot) = self
                .entries
                .values()
                .map(|p| p.player.control.snapshot())
                .find(|s| s.state == PlaybackState::Playing)
        {
            NowPlayingBanner::draw(canvas, &snapshot);
        }
    }
}
pub fn quality_for_height(height: f32) -> u32 {
    if height > 720. {
        1080
    } else if height > 480. {
        720
    } else if height > 360. {
        480
    } else {
        360
    }
}
