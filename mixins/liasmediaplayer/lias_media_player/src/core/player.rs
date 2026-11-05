//! Per-player actor. Blocking resolution, subprocess ownership and CPAL stay off the UI thread.
use super::{
    audio::{AudioClock, AudioFactory, AudioSink, CpalAudioSink},
    ffmpeg::{self, FfmpegSession, MediaProbe, SessionOptions, VideoFrame},
    process::Cancellation,
    resolver::{ResolveOptions, ResolvedMedia, ResolverChain},
    sources::MediaKind,
    tools::{Tool, ToolManager},
};
use parking_lot::{Mutex, RwLock};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackState {
    Idle,
    Resolving,
    Buffering,
    Playing,
    Paused,
    Ended,
    Failed,
}
#[derive(Clone, Debug)]
pub struct PlayerSnapshot {
    pub state: PlaybackState,
    pub source_url: String,
    pub title: String,
    pub position_secs: f64,
    pub duration_secs: Option<f64>,
    pub is_live: bool,
    pub video_size: Option<(u32, u32)>,
    pub error: Option<String>,
    pub speed: f64,
    pub volume: f32,
    pub looping: bool,
    pub target_height: u32,
    pub fps: u32,
    pub sound_only: bool,
    pub queue: Vec<String>,
    pub queue_titles: Vec<String>,
    pub shuffle: bool,
    pub repeat_all: bool,
    pub has_previous: bool,
    pub generation: u64,
}
impl Default for PlayerSnapshot {
    fn default() -> Self {
        Self {
            state: PlaybackState::Idle,
            source_url: String::new(),
            title: String::new(),
            position_secs: 0.,
            duration_secs: None,
            is_live: false,
            video_size: None,
            error: None,
            speed: 1.,
            volume: 1.,
            looping: false,
            target_height: 720,
            fps: 30,
            sound_only: false,
            queue: Vec::new(),
            queue_titles: Vec::new(),
            shuffle: false,
            repeat_all: false,
            has_previous: false,
            generation: 0,
        }
    }
}
#[derive(Default)]
pub struct PlayQueue {
    entries: VecDeque<String>,
    history: VecDeque<String>,
    pub shuffle: bool,
    pub repeat_all: bool,
}
impl PlayQueue {
    pub fn enqueue(&mut self, url: String) -> Result<(), String> {
        if self.entries.len() >= 500 {
            return Err("Queue is full (500 entries)".into());
        }
        super::sources::web_url(&url).ok_or("Expected an HTTP(S) media URL")?;
        self.entries.push_back(url);
        Ok(())
    }
    pub fn enqueue_batch(&mut self, urls: Vec<String>) -> Result<(), String> {
        if urls.len() > 500usize.saturating_sub(self.entries.len()) {
            return Err("Queue is full (500 entries)".into());
        }
        if urls.iter().any(|u| super::sources::web_url(u).is_none()) {
            return Err("Expected HTTP(S) media URLs".into());
        }
        self.entries.extend(urls);
        Ok(())
    }
    pub fn advance(&mut self) -> Option<String> {
        self.entries.pop_front()
    }
    pub fn remove(&mut self, index: usize) -> Option<String> {
        self.entries.remove(index)
    }
    pub fn entries(&self) -> Vec<String> {
        self.entries.iter().cloned().collect()
    }
    pub fn clear(&mut self) {
        self.entries.clear();
        self.history.clear();
    }
    pub fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if on {
            shuffle_slice(self.entries.make_contiguous());
        }
    }
    pub fn move_entry(&mut self, from: usize, to: usize) {
        if from < self.entries.len() && to < self.entries.len() && from != to {
            let item = self.entries.remove(from).unwrap();
            self.entries.insert(to, item);
        }
    }
    pub fn remember(&mut self, current: &str) {
        if !current.is_empty() {
            if self.history.len() >= 500 {
                self.history.pop_front();
            }
            self.history.push_back(current.to_owned());
        }
    }
    pub fn next_track(&mut self, current: &str) -> Option<String> {
        if self.entries.is_empty() && !self.repeat_all {
            return None;
        }
        self.remember(current);
        if self.entries.is_empty() {
            self.entries = std::mem::take(&mut self.history);
            if self.shuffle {
                shuffle_slice(self.entries.make_contiguous());
                if self.entries.len() > 1 && self.entries.front().is_some_and(|s| s == current) {
                    self.entries.swap(0, 1);
                }
            }
        }
        self.advance()
    }
    pub fn previous(&mut self, current: &str) -> Option<String> {
        if self.history.is_empty() || self.entries.len() >= 500 {
            return None;
        }
        if !current.is_empty() {
            self.entries.push_front(current.to_owned());
        }
        self.history.pop_back()
    }
    pub fn has_previous(&self) -> bool {
        !self.history.is_empty()
    }
}
/// Small independent Fisher–Yates shuffle; no new native build dependency.
pub fn shuffle_slice<T>(items: &mut [T]) {
    static SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let mut seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        ^ SEQUENCE.fetch_add(1, Ordering::Relaxed);
    seed |= 1;
    for i in (1..items.len()).rev() {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        items.swap(i, seed as usize % (i + 1));
    }
}
#[derive(Clone, Debug)]
pub enum PlayerCommand {
    Load(String),
    Enqueue(String),
    EnqueueBatch(Vec<String>),
    Play,
    Pause,
    Toggle,
    Seek(f64),
    SetSpeed(f64),
    SetVolume(f32),
    SetLoop(bool),
    SetRepeatAll(bool),
    CycleRepeat,
    SetShuffle(bool),
    SetTitles(Vec<(String, String)>),
    Previous,
    JumpQueue(usize),
    MoveQueue(usize, usize),
    ClearQueue,
    SetTargetHeight(u32),
    SetFps(u32),
    SetVideo(bool),
    Next,
    RemoveQueue(usize),
    Retry,
    Suspend,
    Stop,
}
impl PlayerCommand {
    fn interrupts(&self) -> bool {
        matches!(
            self,
            Self::Load(_)
                | Self::Seek(_)
                | Self::SetSpeed(_)
                | Self::SetTargetHeight(_)
                | Self::SetFps(_)
                | Self::SetVideo(_)
                | Self::Next
                | Self::Previous
                | Self::JumpQueue(_)
                | Self::Retry
                | Self::Suspend
                | Self::Stop
        )
    }
}
struct Message {
    command: PlayerCommand,
    cancel: Option<Cancellation>,
}
pub struct PlayerControl {
    tx: SyncSender<Message>,
    active: Mutex<Cancellation>,
    stop: Cancellation,
    suspend_requested: AtomicBool,
    snapshot: RwLock<Arc<PlayerSnapshot>>,
    frame: Mutex<Option<Arc<VideoFrame>>>,
}
impl PlayerControl {
    pub fn send(&self, command: PlayerCommand) -> Result<(), String> {
        // Reserve the new operation only after it was accepted by the bounded mailbox.
        let mut active = self.active.lock();
        if matches!(command, PlayerCommand::Suspend) {
            active.cancel();
            self.suspend_requested.store(true, Ordering::Release);
            return Ok(());
        }
        let cancel = command.interrupts().then(Cancellation::default);
        self.tx
            .try_send(Message {
                command,
                cancel: cancel.clone(),
            })
            .map_err(|_| "Player is busy or closed")?;
        if let Some(cancel) = cancel {
            active.cancel();
            *active = cancel;
        }
        Ok(())
    }
    pub fn snapshot(&self) -> Arc<PlayerSnapshot> {
        self.snapshot.read().clone()
    }
    pub fn take_frame(&self) -> Option<Arc<VideoFrame>> {
        self.frame.lock().take()
    }
    pub fn close(&self) {
        self.stop.cancel();
        self.active.lock().cancel();
    }
}
pub struct Player {
    pub control: Arc<PlayerControl>,
    worker: Option<JoinHandle<()>>,
}
impl Player {
    pub fn new(tools: ToolManager, audio_only: bool) -> Self {
        Self::with_audio_factory(tools, audio_only, CpalAudioSink::factory())
    }
    pub fn with_audio_factory(tools: ToolManager, audio_only: bool, factory: AudioFactory) -> Self {
        let (tx, rx) = mpsc::sync_channel(64);
        let control = Arc::new(PlayerControl {
            tx,
            active: Mutex::new(Cancellation::default()),
            stop: Cancellation::default(),
            suspend_requested: AtomicBool::new(false),
            snapshot: RwLock::new(Arc::new(PlayerSnapshot::default())),
            frame: Mutex::new(None),
        });
        let actor_control = control.clone();
        let worker = thread::Builder::new()
            .name("lia-player".into())
            .spawn(move || {
                Actor::new(actor_control, tools, audio_only, factory).run(rx);
            })
            .expect("start player actor");
        Self {
            control,
            worker: Some(worker),
        }
    }
    /// Explicit shutdown barrier for tests/hosts that are not on their render thread.
    pub fn join(mut self) {
        self.control.close();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.control.close();
    }
}
struct ActiveSession {
    ffmpeg: FfmpegSession,
    sink: Option<Box<dyn AudioSink>>,
    clock: Arc<AudioClock>,
    started: bool,
    wall_position: f64,
    last_tick: Instant,
    last_progress: Instant,
    last_position: f64,
    last_video_pts: Option<f64>,
}
struct Actor {
    control: Arc<PlayerControl>,
    resolver: ResolverChain,
    audio_only: bool,
    factory: AudioFactory,
    snapshot: PlayerSnapshot,
    queue: PlayQueue,
    titles: BTreeMap<String, String>,
    media: Option<ResolvedMedia>,
    probe: Option<MediaProbe>,
    session: Option<ActiveSession>,
    cancel: Cancellation,
    paused: bool,
    video: bool,
    retried_403: bool,
}
impl Actor {
    fn new(
        control: Arc<PlayerControl>,
        tools: ToolManager,
        audio_only: bool,
        factory: AudioFactory,
    ) -> Self {
        let cancel = control.active.lock().clone();
        Self {
            control,
            resolver: ResolverChain::new(tools),
            audio_only,
            factory,
            snapshot: PlayerSnapshot::default(),
            queue: PlayQueue::default(),
            titles: BTreeMap::new(),
            media: None,
            probe: None,
            session: None,
            cancel,
            paused: false,
            video: !audio_only,
            retried_403: false,
        }
    }
    fn publish(&mut self) {
        self.snapshot.queue = self.queue.entries();
        self.snapshot.queue_titles = self
            .snapshot
            .queue
            .iter()
            .map(|url| {
                self.titles
                    .get(url)
                    .cloned()
                    .unwrap_or_else(|| super::library::short_title(url))
            })
            .collect();
        self.snapshot.shuffle = self.queue.shuffle;
        self.snapshot.repeat_all = self.queue.repeat_all;
        self.snapshot.has_previous = self.queue.has_previous();
        *self.control.snapshot.write() = Arc::new(self.snapshot.clone());
    }
    fn run(mut self, rx: Receiver<Message>) {
        while !self.control.stop.is_cancelled() {
            if self.control.suspend_requested.swap(false, Ordering::AcqRel) {
                // Suspension is never rejected by a full command mailbox. Discard work
                // queued before the unload while holding the same lock as send().
                {
                    let mut active = self.control.active.lock();
                    while rx.try_recv().is_ok() {}
                    self.cancel = Cancellation::default();
                    *active = self.cancel.clone();
                }
                self.session = None;
                self.paused = true;
                self.snapshot.state = PlaybackState::Paused;
                self.publish();
            }
            match rx.recv_timeout(Duration::from_millis(10)) {
                Ok(message) => {
                    if let Some(cancel) = message.cancel {
                        self.cancel = cancel;
                    }
                    let queue_edit = matches!(
                        message.command,
                        PlayerCommand::Enqueue(_) | PlayerCommand::EnqueueBatch(_)
                    );
                    if let Err(e) = self.command(message.command) {
                        if queue_edit && self.session.is_some() {
                            self.snapshot.error = Some(e);
                        } else {
                            self.fail(e);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if !self.control.stop.is_cancelled() && !self.cancel.is_cancelled() {
                if let Err(e) = self.tick() {
                    self.fail(e);
                }
                self.publish();
            }
        }
    }
    fn fail(&mut self, error: String) {
        self.session = None;
        if self.cancel.is_cancelled() || self.control.stop.is_cancelled() {
            return;
        }
        if error.contains("403") && !self.retried_403 {
            self.retried_403 = true;
            self.resolver.on_http_status(&self.snapshot.source_url, 403);
            self.media = None;
            self.probe = None;
            if let Err(e) = self.restart() {
                self.snapshot.error = Some(e);
                self.snapshot.state = PlaybackState::Failed;
            }
        } else {
            self.snapshot.error = Some(error);
            self.snapshot.state = PlaybackState::Failed;
        }
    }
    fn load(&mut self, url: String) -> Result<(), String> {
        super::sources::web_url(&url).ok_or("Expected an HTTP(S) media URL")?;
        self.snapshot.source_url = url.clone();
        self.snapshot.title = url;
        self.snapshot.position_secs = 0.;
        self.snapshot.duration_secs = None;
        self.snapshot.is_live = false;
        self.snapshot.video_size = None;
        self.media = None;
        self.probe = None;
        self.retried_403 = false;
        self.paused = false;
        self.restart()
    }
    fn restart(&mut self) -> Result<(), String> {
        self.session = None;
        self.snapshot.error = None;
        self.snapshot.generation += 1;
        *self.control.frame.lock() = None;
        if self.snapshot.source_url.is_empty() {
            return Ok(());
        }
        if self.cancel.is_cancelled() {
            return Err("Playback cancelled".into());
        }
        if self.media.is_none() {
            self.snapshot.state = PlaybackState::Resolving;
            self.publish();
            let media = self
                .resolver
                .resolve(
                    &self.snapshot.source_url,
                    if self.audio_only {
                        MediaKind::Audio
                    } else {
                        MediaKind::Video
                    },
                    ResolveOptions {
                        max_height: self.snapshot.target_height,
                        audio_only: self.audio_only,
                    },
                    &self.cancel,
                )
                .map_err(|e| e.to_string())?;
            self.snapshot.title = media.title.clone().unwrap_or_else(|| {
                self.titles
                    .get(&self.snapshot.source_url)
                    .cloned()
                    .unwrap_or_else(|| super::library::short_title(&self.snapshot.source_url))
            });
            self.titles.insert(
                self.snapshot.source_url.clone(),
                self.snapshot.title.clone(),
            );
            self.snapshot.duration_secs = media.duration;
            self.snapshot.is_live = media.is_live;
            self.media = Some(media);
        }
        let media = self.media.as_ref().unwrap();
        if self.probe.is_none() {
            let path = self
                .resolver
                .tools
                .require(Tool::Ffmpeg, &self.cancel)
                .map_err(|e| e.to_string())?;
            let primary = media
                .video
                .as_ref()
                .or(media.audio.as_ref())
                .ok_or("No media streams")?;
            let mut probe =
                ffmpeg::probe(&path, primary, &self.cancel).map_err(|e| e.to_string())?;
            if let Some((video, audio)) = media.video.as_ref().zip(media.audio.as_ref())
                && video.url != audio.url
            {
                let audio_probe =
                    ffmpeg::probe(&path, audio, &self.cancel).map_err(|e| e.to_string())?;
                probe.audio = audio_probe.audio;
                probe.duration = probe
                    .duration
                    .into_iter()
                    .chain(audio_probe.duration)
                    .reduce(f64::max);
            }
            if self.audio_only && !probe.audio {
                return Err("This media has no audio track".into());
            }
            self.snapshot.duration_secs = self
                .snapshot
                .duration_secs
                .into_iter()
                .chain(probe.duration)
                .reduce(f64::max);
            self.probe = Some(probe);
        }
        if self.cancel.is_cancelled() {
            return Err("Playback cancelled".into());
        }
        self.snapshot.state = if self.paused {
            PlaybackState::Paused
        } else {
            PlaybackState::Buffering
        };
        self.publish();
        let media = self.media.as_ref().unwrap();
        let probe = self.probe.as_ref().unwrap();
        // A silent video must retain video decoding when hidden to keep a valid timeline.
        let options = SessionOptions {
            start: if self.snapshot.is_live {
                0.
            } else {
                self.snapshot.position_secs
            },
            speed: self.snapshot.speed,
            fps: self.snapshot.fps,
            video: self.video || !probe.audio && !self.audio_only,
        };
        let (ffmpeg, pcm) =
            FfmpegSession::start(&self.resolver.tools, media, probe, options, &self.cancel)
                .map_err(|e| e.to_string())?;
        let clock = Arc::new(AudioClock::default());
        clock.set_gain(self.snapshot.volume);
        let sink = pcm
            .map(|pcm| (self.factory)(pcm, clock.clone()))
            .transpose()?;
        self.snapshot.position_secs = options.start;
        self.session = Some(ActiveSession {
            ffmpeg,
            sink,
            clock,
            started: false,
            wall_position: options.start,
            last_tick: Instant::now(),
            last_progress: Instant::now(),
            last_position: options.start,
            last_video_pts: None,
        });
        Ok(())
    }
    fn command(&mut self, command: PlayerCommand) -> Result<(), String> {
        match command {
            PlayerCommand::Load(url) => {
                self.queue.clear();
                self.load(url)?;
            }
            PlayerCommand::Enqueue(url) => {
                if matches!(
                    self.snapshot.state,
                    PlaybackState::Idle | PlaybackState::Ended
                ) {
                    self.load(url)?;
                } else {
                    self.queue.enqueue(url)?;
                }
            }
            PlayerCommand::EnqueueBatch(urls) => {
                self.queue.enqueue_batch(urls)?;
                if matches!(
                    self.snapshot.state,
                    PlaybackState::Idle | PlaybackState::Ended
                ) && let Some(url) = self.queue.advance()
                {
                    self.load(url)?;
                }
            }
            PlayerCommand::Play => {
                self.paused = false;
                if self.session.is_none() {
                    if self.snapshot.state == PlaybackState::Ended {
                        self.snapshot.position_secs = 0.;
                    }
                    self.restart()?;
                }
            }
            PlayerCommand::Pause => self.paused = true,
            PlayerCommand::Toggle => {
                if self.paused
                    || matches!(
                        self.snapshot.state,
                        PlaybackState::Ended | PlaybackState::Failed
                    )
                {
                    self.command(PlayerCommand::Play)?;
                } else {
                    self.paused = true;
                }
            }
            PlayerCommand::Seek(value) if value.is_finite() && !self.snapshot.is_live => {
                self.snapshot.position_secs = value.max(0.).min(
                    self.snapshot
                        .duration_secs
                        .map_or(f64::MAX, |d| (d - 0.05).max(0.)),
                );
                self.restart()?;
            }
            PlayerCommand::SetSpeed(value) if value.is_finite() => {
                self.snapshot.speed = value.clamp(0.25, 4.);
                self.restart()?;
            }
            PlayerCommand::SetVolume(value) if value.is_finite() => {
                self.snapshot.volume = value.clamp(0., 1.);
                if let Some(s) = &self.session {
                    s.clock.set_gain(self.snapshot.volume);
                }
            }
            PlayerCommand::SetLoop(value) => {
                self.snapshot.looping = value;
                if value {
                    self.queue.repeat_all = false;
                }
            }
            PlayerCommand::SetRepeatAll(value) => {
                self.queue.repeat_all = value;
                if value {
                    self.snapshot.looping = false;
                }
            }
            PlayerCommand::CycleRepeat => {
                if self.snapshot.looping {
                    self.snapshot.looping = false;
                } else if self.queue.repeat_all {
                    self.queue.repeat_all = false;
                    self.snapshot.looping = true;
                } else {
                    self.queue.repeat_all = true;
                }
            }
            PlayerCommand::SetShuffle(on) => self.queue.set_shuffle(on),
            PlayerCommand::SetTitles(titles) => {
                for (url, title) in titles.into_iter().take(500) {
                    if self.titles.len() >= 2000 {
                        self.titles.clear();
                    }
                    if !title.is_empty() {
                        let title: String = title.chars().take(256).collect();
                        if url == self.snapshot.source_url {
                            self.snapshot.title = title.clone();
                        }
                        self.titles.insert(url, title);
                    }
                }
            }
            PlayerCommand::Previous => {
                if let Some(url) = self.queue.previous(&self.snapshot.source_url) {
                    self.load(url)?;
                } else if self.session.is_some() {
                    self.restart()?;
                }
            }
            PlayerCommand::JumpQueue(i) => {
                if let Some(url) = self.queue.remove(i) {
                    self.queue.remember(&self.snapshot.source_url);
                    self.load(url)?;
                } else if self.session.is_some() {
                    self.restart()?;
                }
            }
            PlayerCommand::MoveQueue(from, to) => self.queue.move_entry(from, to),
            PlayerCommand::ClearQueue => self.queue.clear(),
            PlayerCommand::SetTargetHeight(value) if [360, 480, 720, 1080].contains(&value) => {
                if value != self.snapshot.target_height {
                    self.snapshot.target_height = value;
                    self.media = None;
                    self.probe = None;
                    self.restart()?;
                }
            }
            PlayerCommand::SetFps(value) if [30, 60].contains(&value) => {
                self.snapshot.fps = value;
                self.restart()?;
            }
            PlayerCommand::SetVideo(value) => {
                if !self.audio_only && self.video != value {
                    self.video = value;
                    self.snapshot.sound_only = !value;
                    self.restart()?;
                }
            }
            PlayerCommand::Next => {
                if let Some(url) = self.queue.next_track(&self.snapshot.source_url) {
                    self.load(url)?;
                } else {
                    self.session = None;
                    self.snapshot.state = PlaybackState::Ended;
                }
            }
            PlayerCommand::RemoveQueue(index) => {
                self.queue.remove(index);
            }
            PlayerCommand::Retry => {
                self.media = None;
                self.probe = None;
                self.retried_403 = false;
                self.restart()?;
            }
            PlayerCommand::Suspend => {
                self.paused = true;
                self.session = None;
                self.snapshot.state = PlaybackState::Paused;
            }
            PlayerCommand::Stop => {
                self.session = None;
                self.queue.clear();
                self.snapshot.state = PlaybackState::Idle;
                self.snapshot.position_secs = 0.;
            }
            _ => {}
        }
        Ok(())
    }
    fn tick(&mut self) -> Result<(), String> {
        let Some(s) = &mut self.session else {
            return Ok(());
        };
        let now = Instant::now();
        let dt = now.duration_since(s.last_tick).as_secs_f64();
        s.last_tick = now;
        let status = s.ffmpeg.poll().map_err(|e| e.to_string())?;
        if s.clock.failed.load(Ordering::Acquire) {
            return Err(
                "Audio output device disconnected. Retry after choosing an output device.".into(),
            );
        }
        let audio_done = s.ffmpeg.audio_done.load(Ordering::Acquire);
        let received = s.ffmpeg.received_audio.load(Ordering::Acquire);
        let video_done = s.ffmpeg.video_done.load(Ordering::Acquire);
        if !s.started {
            s.started = if s.sink.is_some() {
                received >= 2400 || audio_done && received > 0
            } else {
                !s.ffmpeg.frames.is_empty()
            };
        }
        s.clock.pause(self.paused || !s.started);
        if let Some(sink) = &mut s.sink {
            sink.poll();
        }
        if self.paused {
            s.last_progress = now;
            self.snapshot.state = PlaybackState::Paused;
            return Ok(());
        }
        let consumed = s.clock.played_frames.load(Ordering::Acquire);
        let audio_drained = audio_done && consumed >= received;
        if s.started {
            self.retried_403 = false;
            self.snapshot.state = PlaybackState::Playing;
            if s.sink.is_some() && !audio_drained {
                self.snapshot.position_secs =
                    s.wall_position + s.clock.seconds() * self.snapshot.speed;
            } else {
                self.snapshot.position_secs += dt.min(0.1) * self.snapshot.speed;
            }
            if let Some(frame) = s.ffmpeg.frames.take_due(self.snapshot.position_secs) {
                self.snapshot.video_size = Some((frame.width, frame.height));
                s.last_video_pts = Some(frame.pts);
                s.last_progress = now;
                *self.control.frame.lock() = Some(frame);
            }
        }
        if s.sink.is_some()
            && !audio_drained
            && (self.snapshot.position_secs - s.last_position).abs() > 0.001
        {
            s.last_progress = now;
            s.last_position = self.snapshot.position_secs;
        }
        if now.duration_since(s.last_progress) > Duration::from_secs(30) {
            return Err("Playback stalled for 30 seconds. Retry to reconnect.".into());
        }
        if status == Some(true)
            && audio_drained
            && video_done
            && s.ffmpeg.frames.is_empty()
            && s.last_video_pts.is_none_or(|pts| {
                self.snapshot.position_secs >= pts + self.snapshot.speed / self.snapshot.fps as f64
            })
        {
            self.session = None;
            if self.snapshot.looping && !self.snapshot.is_live {
                self.snapshot.position_secs = 0.;
                self.restart()?;
            } else if let Some(url) = self.queue.next_track(&self.snapshot.source_url) {
                self.load(url)?;
            } else {
                self.snapshot.state = PlaybackState::Ended;
                if let Some(duration) = self.snapshot.duration_secs {
                    self.snapshot.position_secs = duration;
                }
            }
        }
        Ok(())
    }
}
