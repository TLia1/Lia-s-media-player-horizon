//! One FFmpeg process, bounded PAM frames on stdout and stereo s16le over loopback.
use super::{
    audio::{PcmConsumer, PcmProducer, PcmRing, SAMPLE_RATE},
    image::ffmpeg_image::pam_header,
    process::{Cancellation, ManagedProcess},
    resolver::{ResolvedMedia, StreamUrl},
    tools::{Tool, ToolManager},
};
use parking_lot::Mutex;
use std::{
    collections::VecDeque,
    io::{self, BufRead, BufReader, Read},
    net::TcpListener,
    path::Path,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct FramePool {
    free: Mutex<Vec<Vec<u8>>>,
}
impl FramePool {
    fn acquire(&self, bytes: usize) -> Vec<u8> {
        let mut v = self.free.lock().pop().unwrap_or_default();
        v.resize(bytes, 0);
        v
    }
    fn recycle(&self, pixels: Vec<u8>) {
        let mut free = self.free.lock();
        if free.len() < 10 {
            free.push(pixels);
        }
    }
}
pub struct VideoFrame {
    pub width: u32,
    pub height: u32,
    pub pts: f64,
    pixels: Vec<u8>,
    pool: Arc<FramePool>,
}
impl VideoFrame {
    pub fn pixels(&self) -> &[u8] {
        &self.pixels
    }
}
impl Drop for VideoFrame {
    fn drop(&mut self) {
        self.pool.recycle(std::mem::take(&mut self.pixels));
    }
}
#[derive(Default)]
pub struct FrameRing {
    frames: Mutex<VecDeque<Arc<VideoFrame>>>,
}
impl FrameRing {
    pub const CAPACITY: usize = 8;
    pub fn push(&self, frame: Arc<VideoFrame>, cancel: &Cancellation) -> bool {
        while !cancel.is_cancelled() {
            {
                let mut frames = self.frames.lock();
                if frames.len() < Self::CAPACITY {
                    frames.push_back(frame);
                    return true;
                }
            }
            thread::sleep(Duration::from_millis(4));
        }
        false
    }
    /// Drop late frames, retain future frames, return only the newest frame due.
    pub fn take_due(&self, clock: f64) -> Option<Arc<VideoFrame>> {
        let mut frames = self.frames.lock();
        let mut result = None;
        while frames.front().is_some_and(|f| f.pts <= clock + 0.001) {
            result = frames.pop_front();
        }
        result
    }
    pub fn is_empty(&self) -> bool {
        self.frames.lock().is_empty()
    }
}
pub struct PamReader<R> {
    reader: R,
    pool: Arc<FramePool>,
    index: u64,
    start: f64,
    speed: f64,
    fps: u32,
}
impl<R: BufRead> PamReader<R> {
    pub fn new(
        reader: R,
        start: f64,
        speed: f64,
        fps: u32,
        pool: Arc<FramePool>,
    ) -> io::Result<Self> {
        if !start.is_finite()
            || start < 0.
            || !speed.is_finite()
            || !(0.25..=4.).contains(&speed)
            || ![30, 60].contains(&fps)
        {
            return Err(io::Error::other("Invalid video timeline"));
        }
        Ok(Self {
            reader,
            pool,
            index: 0,
            start,
            speed,
            fps,
        })
    }
    pub fn read_frame(&mut self) -> io::Result<Option<Arc<VideoFrame>>> {
        let Some((width, height)) = pam_header(&mut self.reader)? else {
            return Ok(None);
        };
        if width == 0 || height == 0 || width > 512 || height > 512 {
            return Err(io::Error::other("Video frame exceeds 512x512 atlas limit"));
        }
        let mut pixels = self.pool.acquire(width as usize * height as usize * 4);
        self.reader.read_exact(&mut pixels)?;
        let pts = self.start + self.index as f64 / self.fps as f64 * self.speed;
        self.index += 1;
        Ok(Some(Arc::new(VideoFrame {
            width,
            height,
            pts,
            pixels,
            pool: self.pool.clone(),
        })))
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SessionOptions {
    pub start: f64,
    pub speed: f64,
    pub fps: u32,
    pub video: bool,
}
impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            start: 0.,
            speed: 1.,
            fps: 30,
            video: true,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct MediaProbe {
    pub video: bool,
    pub audio: bool,
    pub duration: Option<f64>,
}
fn input(command: &mut Command, stream: &StreamUrl, start: f64) -> io::Result<()> {
    if super::sources::web_url(&stream.url).is_none() {
        return Err(io::Error::other("Expected HTTP(S) stream"));
    }
    command.args([
        "-protocol_whitelist",
        "http,https,tcp,tls,crypto,hls,dash,data",
        "-rw_timeout",
        "15000000",
        "-reconnect",
        "1",
        "-reconnect_streamed",
        "1",
        "-reconnect_delay_max",
        "5",
    ]);
    if start > 0. {
        command.arg("-ss").arg(format!("{start:.6}"));
    }
    if !stream.headers.is_empty() {
        let mut headers = String::new();
        for (key, value) in &stream.headers {
            if key.is_empty()
                || !key.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
                || value.contains(['\r', '\n', '\0'])
            {
                return Err(io::Error::other("Invalid stream header"));
            }
            headers.push_str(&format!("{key}: {value}\r\n"));
        }
        command.arg("-headers").arg(headers);
    }
    command.arg("-i").arg(&stream.url);
    Ok(())
}
fn base(path: &Path) -> Command {
    let mut command = Command::new(path);
    command.args([
        "-hide_banner",
        "-nostdin",
        "-threads",
        "1",
        "-filter_threads",
        "1",
    ]);
    command
}
/// FFmpeg itself supplies stream discovery, so no separate ffprobe binary is required.
pub fn probe(path: &Path, stream: &StreamUrl, cancel: &Cancellation) -> io::Result<MediaProbe> {
    let mut cmd = base(path);
    input(&mut cmd, stream, 0.)?;
    let mut child = ManagedProcess::spawn(&mut cmd)?;
    let started = Instant::now();
    loop {
        if cancel.is_cancelled() {
            return Err(io::Error::other("Playback cancelled"));
        }
        if child.try_wait()?.is_some() {
            break;
        }
        if started.elapsed() > Duration::from_secs(25) {
            return Err(io::Error::other("FFmpeg media probe timed out"));
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.terminate();
    let log = child.stderr();
    let video = log
        .lines()
        .any(|s| s.contains("Stream #") && s.contains("Video:") && !s.contains("attached pic"));
    let audio = log
        .lines()
        .any(|s| s.contains("Stream #") && s.contains("Audio:"));
    if !video && !audio {
        log::warn!("FFmpeg could not read media: {log}");
        return Err(io::Error::other(format!(
            "FFmpeg could not read media:\n{log}"
        )));
    }
    let duration = log
        .split("Duration: ")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .and_then(|s| {
            let n: Vec<f64> = s.split(':').filter_map(|n| n.trim().parse().ok()).collect();
            (n.len() == 3).then(|| n[0] * 3600. + n[1] * 60. + n[2])
        })
        .filter(|v| v.is_finite() && *v >= 0.);
    Ok(MediaProbe {
        video,
        audio,
        duration,
    })
}
pub fn command(
    path: &Path,
    media: &ResolvedMedia,
    probe: &MediaProbe,
    options: SessionOptions,
    port: u16,
) -> io::Result<Command> {
    if !options.start.is_finite()
        || options.start < 0.
        || !options.speed.is_finite()
        || !(0.25..=4.).contains(&options.speed)
        || ![30, 60].contains(&options.fps)
    {
        return Err(io::Error::other("Invalid playback options"));
    }
    let first = media
        .video
        .as_ref()
        .or(media.audio.as_ref())
        .ok_or_else(|| io::Error::other("Missing stream"))?;
    let mut cmd = base(path);
    cmd.args(["-loglevel", "error"]);
    input(&mut cmd, first, options.start)?;
    let split = media
        .video
        .as_ref()
        .zip(media.audio.as_ref())
        .filter(|(v, a)| v.url != a.url);
    if let Some((_, audio)) = split {
        input(&mut cmd, audio, options.start)?;
    }
    if options.video && probe.video {
        cmd.args(["-map", "0:v:0", "-an", "-sn", "-dn", "-vf"])
            .arg(format!("setpts=(PTS-STARTPTS)/{},scale=512:512:force_original_aspect_ratio=decrease:flags=fast_bilinear", options.speed))
            .args(["-fps_mode", "cfr", "-r"]).arg(options.fps.to_string())
            .args(["-threads", "1", "-pix_fmt", "rgba", "-f", "image2pipe", "-c:v", "pam", "pipe:1"]);
    }
    if probe.audio {
        let mut filters = format!(
            "asetpts=PTS-STARTPTS,atempo={},atempo={}",
            options.speed.sqrt(),
            options.speed.sqrt()
        );
        // FFmpeg closes output sockets only after all outputs finish. Without padding,
        // short audio can freeze the master clock while the bounded video pipe waits
        // for future frames to become due, preventing either output from reaching EOF.
        if !media.is_live && probe.video {
            let duration = probe
                .duration
                .into_iter()
                .chain(media.duration)
                .filter(|d| d.is_finite())
                .reduce(f64::max);
            if let Some(duration) = duration.filter(|d| *d > options.start) {
                filters.push_str(&format!(
                    ",apad=whole_dur={:.6}",
                    (duration - options.start) / options.speed
                ));
            }
        }
        cmd.args([
            "-map",
            if split.is_some() { "1:a:0" } else { "0:a:0" },
            "-vn",
            "-sn",
            "-dn",
            "-af",
        ])
        .arg(filters)
        .args([
            "-ac",
            "2",
            "-ar",
            "48000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "s16le",
        ])
        .arg(format!("tcp://127.0.0.1:{port}"));
    }
    if !(probe.audio || options.video && probe.video) {
        return Err(io::Error::other("No playable streams for this player"));
    }
    Ok(cmd)
}
pub struct FfmpegSession {
    process: ManagedProcess,
    pub frames: Arc<FrameRing>,
    pub audio_done: Arc<AtomicBool>,
    pub video_done: Arc<AtomicBool>,
    pub received_audio: Arc<AtomicU64>,
    error: Arc<Mutex<Option<String>>>,
    cancel: Cancellation,
    workers: Vec<JoinHandle<()>>,
    status: Option<bool>,
}
impl FfmpegSession {
    pub fn start(
        tools: &ToolManager,
        media: &ResolvedMedia,
        probe: &MediaProbe,
        options: SessionOptions,
        cancel: &Cancellation,
    ) -> io::Result<(Self, Option<PcmConsumer>)> {
        let path = tools.require(Tool::Ffmpeg, cancel)?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let mut process = ManagedProcess::spawn(&mut command(
            &path,
            media,
            probe,
            options,
            listener.local_addr()?.port(),
        )?)?;
        let stdout = process.take_stdout().unwrap();
        let frames = Arc::new(FrameRing::default());
        let audio_done = Arc::new(AtomicBool::new(!probe.audio));
        let video_done = Arc::new(AtomicBool::new(!(options.video && probe.video)));
        let received_audio = Arc::new(AtomicU64::new(0));
        let error = Arc::new(Mutex::new(None));
        let stop = Cancellation::default();
        let mut workers = Vec::new();
        if options.video && probe.video {
            let ring = frames.clone();
            let done = video_done.clone();
            let stop = stop.clone();
            let err = error.clone();
            workers.push(thread::spawn(move || {
                let result = (|| -> io::Result<()> {
                    let mut reader = PamReader::new(
                        BufReader::with_capacity(256 * 1024, stdout),
                        options.start,
                        options.speed,
                        options.fps,
                        Arc::new(FramePool::default()),
                    )?;
                    while !stop.is_cancelled() {
                        let Some(frame) = reader.read_frame()? else {
                            break;
                        };
                        if !ring.push(frame, &stop) {
                            break;
                        }
                    }
                    Ok(())
                })();
                if let Err(e) = result {
                    *err.lock() = Some(e.to_string());
                }
                done.store(true, Ordering::Release);
            }));
        } else {
            drop(stdout);
        }
        let consumer = if probe.audio {
            let (producer, consumer) = PcmRing::bounded(SAMPLE_RATE as usize * 2);
            let stop = stop.clone();
            let done = audio_done.clone();
            let err = error.clone();
            let count = received_audio.clone();
            workers.push(thread::spawn(move || {
                if let Err(e) = read_audio(listener, producer, &stop, &count) {
                    *err.lock() = Some(e.to_string());
                }
                done.store(true, Ordering::Release);
            }));
            Some(consumer)
        } else {
            drop(listener);
            None
        };
        Ok((
            Self {
                process,
                frames,
                audio_done,
                video_done,
                received_audio,
                error,
                cancel: stop,
                workers,
                status: None,
            },
            consumer,
        ))
    }
    pub fn poll(&mut self) -> io::Result<Option<bool>> {
        if self.status.is_none() {
            self.status = self.process.try_wait()?.map(|s| s.success());
        }
        if self.status == Some(false) {
            self.process.terminate();
            let detail = self.process.stderr();
            log::warn!("FFmpeg playback failed: {detail}");
            return Err(io::Error::other(format!("FFmpeg: {detail}")));
        }
        if let Some(e) = self.error.lock().clone() {
            return Err(io::Error::other(e));
        }
        Ok(self.status)
    }
}
impl Drop for FfmpegSession {
    fn drop(&mut self) {
        self.cancel.cancel();
        self.process.terminate();
        for worker in self.workers.drain(..) {
            let _ = worker.join();
        }
    }
}
fn read_audio(
    listener: TcpListener,
    mut producer: PcmProducer,
    stop: &Cancellation,
    count: &AtomicU64,
) -> io::Result<()> {
    let mut stream = loop {
        if stop.is_cancelled() {
            return Ok(());
        }
        match listener.accept() {
            Ok((s, _)) => break s,
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5))
            }
            Err(e) => return Err(e),
        }
    };
    stream.set_read_timeout(Some(Duration::from_millis(100)))?;
    let mut bytes = [0u8; 8192];
    let mut partial = [0u8; 4];
    let mut used = 0;
    while !stop.is_cancelled() {
        let n = match stream.read(&mut bytes) {
            Ok(0) => {
                if used != 0 {
                    return Err(io::Error::other("Truncated PCM frame"));
                }
                break;
            }
            Ok(n) => n,
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                continue;
            }
            Err(e) => return Err(e),
        };
        for &byte in &bytes[..n] {
            partial[used] = byte;
            used += 1;
            if used == 4 {
                let frame = [
                    i16::from_le_bytes([partial[0], partial[1]]),
                    i16::from_le_bytes([partial[2], partial[3]]),
                ];
                while !producer.push(frame) {
                    if stop.is_cancelled() {
                        return Ok(());
                    }
                    thread::sleep(Duration::from_millis(4));
                }
                count.fetch_add(1, Ordering::Release);
                used = 0;
            }
        }
    }
    Ok(())
}
