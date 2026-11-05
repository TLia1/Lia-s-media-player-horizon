use lias_media_player::{
    core::{
        audio::*,
        ffmpeg::*,
        playback::*,
        player::*,
        process::Cancellation,
        resolver::{DirectResolver, StreamUrl},
        sources::MediaKind,
        tools::ToolManager,
    },
    platform::{InputEvent, Key, Modifiers, Rect, horizon_impl::ModAtlasPacker},
    ui::{
        Widget, WindowManager,
        video_window::{PlaybackView, VideoWindow},
        widgets::{EventContext, InputState},
    },
};
use parking_lot::Mutex;
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[test]
fn pcm_ring_wraparound_preserves_stereo_under_contention() {
    let (mut producer, mut consumer) = PcmRing::bounded(17);
    let writer = thread::spawn(move || {
        for i in 0..100_000 {
            let v = i as i16;
            while !producer.push([v, !v]) {
                thread::yield_now();
            }
        }
    });
    for i in 0..100_000 {
        let frame = loop {
            if let Some(frame) = consumer.pop() {
                break frame;
            }
            thread::yield_now();
        };
        let v = i as i16;
        assert_eq!(frame, [v as f32 / 32768., !v as f32 / 32768.]);
    }
    writer.join().unwrap();
    assert!(consumer.pop().is_none());
}
#[test]
fn resampling_clock_excludes_pause_and_underrun_and_smooths_gain() {
    let (mut producer, consumer) = PcmRing::bounded(96_000);
    for _ in 0..48_000 {
        assert!(producer.push([16_384, -16_384]));
    }
    let clock = Arc::new(AudioClock::default());
    let mut renderer = AudioRenderer::new(consumer, clock.clone());
    assert_eq!(renderer.frame(44_100), [0.; 2]);
    assert_eq!(clock.seconds(), 0.);
    clock.pause(false);
    let first = renderer.frame(44_100);
    assert!(first[0] > 0. && first[0] < 0.01);
    for _ in 1..44_100 {
        let frame = renderer.frame(44_100);
        assert_eq!(frame[0], -frame[1]);
    }
    assert!((clock.seconds() - 1.).abs() < 0.0001);
    for _ in 0..1000 {
        renderer.frame(44_100);
    }
    assert_eq!(clock.played_frames.load(Ordering::Acquire), 48_000);
    for _ in 0..1000 {
        assert_eq!(renderer.frame(44_100), [0.; 2]);
    }
    assert_eq!(clock.seconds(), 1.);
    assert!(producer.push([16_384, 16_384]));
    clock.pause(true);
    for _ in 0..100 {
        assert_eq!(renderer.frame(48_000), [0.; 2]);
    }
    assert_eq!(clock.seconds(), 1.);
}
#[test]
fn renderer_handles_mono_multichannel_and_integer_output() {
    let (mut p, c) = PcmRing::bounded(1000);
    for _ in 0..1000 {
        p.push([16000, -16000]);
    }
    let clock = Arc::new(AudioClock::default());
    clock.pause(false);
    let mut renderer = AudioRenderer::new(c, clock);
    let mut mono = [42i16; 256];
    renderer.render(&mut mono, 1, 48_000);
    assert!(mono.iter().all(|s| *s == 0));
    let mut surround = [42f32; 6];
    renderer.render(&mut surround, 6, 48_000);
    assert!(surround[0] > 0.);
    assert_eq!(surround[0], -surround[1]);
    assert_eq!(&surround[2..], &[0.; 4]);
}
fn pam(width: u32, height: u32) -> Vec<u8> {
    let mut v = format!(
        "P7\nWIDTH {width}\nHEIGHT {height}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n"
    )
    .into_bytes();
    v.extend(vec![127; width as usize * height as usize * 4]);
    v
}
#[test]
fn pam_frames_are_bounded_timestamped_and_scheduled() {
    let mut bytes = pam(2, 3);
    bytes.extend(pam(2, 3));
    bytes.extend(pam(2, 3));
    let mut reader = PamReader::new(
        Cursor::new(bytes),
        10.,
        2.,
        30,
        Arc::new(FramePool::default()),
    )
    .unwrap();
    let ring = FrameRing::default();
    let cancel = Cancellation::default();
    for _ in 0..3 {
        assert!(ring.push(reader.read_frame().unwrap().unwrap(), &cancel));
    }
    assert!(reader.read_frame().unwrap().is_none());
    assert!(ring.take_due(9.).is_none());
    let frame = ring.take_due(10.07).unwrap();
    assert!((frame.pts - 10. - 2. / 30.).abs() < 1e-6);
    assert_eq!(frame.pixels().len(), 24);
    assert!(!ring.is_empty());
    assert!(ring.take_due(11.).is_some());
    assert!(ring.is_empty());
    for bytes in [pam(513, 1), pam(0, 1), b"P7\nWIDTH 1\n".to_vec(), {
        let mut p = pam(2, 3);
        p.pop();
        p
    }] {
        let mut reader = PamReader::new(
            Cursor::new(bytes),
            0.,
            1.,
            30,
            Arc::new(FramePool::default()),
        )
        .unwrap();
        assert!(reader.read_frame().is_err());
    }
}
#[test]
fn bounded_frame_queue_unblocks_on_cancellation() {
    let ring = Arc::new(FrameRing::default());
    let cancel = Cancellation::default();
    let frame = PamReader::new(
        Cursor::new(pam(1, 1)),
        0.,
        1.,
        30,
        Arc::new(FramePool::default()),
    )
    .unwrap()
    .read_frame()
    .unwrap()
    .unwrap();
    for _ in 0..FrameRing::CAPACITY {
        assert!(ring.push(frame.clone(), &cancel));
    }
    let c = cancel.clone();
    let r = ring.clone();
    let thread = thread::spawn(move || r.push(frame, &c));
    cancel.cancel();
    assert!(!thread.join().unwrap());
}
#[test]
fn ffmpeg_arguments_preserve_split_headers_and_reject_injection() {
    let mut media =
        DirectResolver::resolve("https://example.org/video.mp4", MediaKind::Video).unwrap();
    media
        .video
        .as_mut()
        .unwrap()
        .headers
        .insert("Referer".into(), "https://example.org/watch".into());
    media.audio = Some(StreamUrl {
        url: "https://example.org/audio.m4a".into(),
        headers: BTreeMap::from([("Cookie".into(), "token=audio".into())]),
    });
    let probe = MediaProbe {
        video: true,
        audio: true,
        duration: None,
    };
    let options = SessionOptions {
        start: 12.5,
        speed: 0.25,
        fps: 60,
        video: true,
    };
    let cmd = command(
        std::path::Path::new("ffmpeg"),
        &media,
        &probe,
        options,
        12345,
    )
    .unwrap();
    let args: Vec<_> = cmd
        .get_args()
        .map(|s| s.to_string_lossy().into_owned())
        .collect();
    assert_eq!(args.iter().filter(|s| *s == "-ss").count(), 2);
    assert!(args.contains(&"1:a:0".into()));
    assert!(args.contains(&"Cookie: token=audio\r\n".into()));
    assert!(args.contains(&"tcp://127.0.0.1:12345".into()));
    assert!(args.contains(&"pipe:1".into()));
    media
        .audio
        .as_mut()
        .unwrap()
        .headers
        .insert("Cookie".into(), "good\r\nBad: injected".into());
    assert!(
        command(
            std::path::Path::new("ffmpeg"),
            &media,
            &probe,
            options,
            12345
        )
        .is_err()
    );
}
#[test]
fn queue_limits_and_order_are_explicit() {
    let mut queue = PlayQueue::default();
    assert!(queue.enqueue("file:///bad".into()).is_err());
    for i in 0..500 {
        queue
            .enqueue(format!("https://example.org/{i}.mp4"))
            .unwrap();
    }
    assert!(
        queue
            .enqueue("https://example.org/extra.mp4".into())
            .is_err()
    );
    assert_eq!(queue.remove(1).unwrap(), "https://example.org/1.mp4");
    assert_eq!(queue.advance().unwrap(), "https://example.org/0.mp4");
    assert_eq!(queue.advance().unwrap(), "https://example.org/2.mp4");
    queue.clear();
    assert!(queue.advance().is_none());
    assert_eq!(
        [300., 400., 600., 900.].map(quality_for_height),
        [360, 480, 720, 1080]
    );
}

#[test]
fn playlist_navigation_reorders_and_repeats_whole_rounds() {
    let urls: Vec<_> = (0..4)
        .map(|i| format!("https://example.org/{i}.mp3"))
        .collect();
    let mut queue = PlayQueue::default();
    queue.enqueue_batch(urls[1..].to_vec()).unwrap();
    queue.move_entry(2, 0);
    assert_eq!(
        queue.entries(),
        vec![urls[3].clone(), urls[1].clone(), urls[2].clone()]
    );
    assert_eq!(queue.next_track(&urls[0]), Some(urls[3].clone()));
    assert_eq!(queue.previous(&urls[3]), Some(urls[0].clone()));
    assert_eq!(queue.next_track(&urls[0]), Some(urls[3].clone()));
    queue.repeat_all = true;
    assert_eq!(queue.next_track(&urls[3]), Some(urls[1].clone()));
    assert_eq!(queue.next_track(&urls[1]), Some(urls[2].clone()));
    assert_eq!(queue.next_track(&urls[2]), Some(urls[0].clone()));
    let mut round = queue.entries();
    round.sort();
    assert_eq!(round, urls[1..]);
    queue.set_shuffle(true);
    let mut current = urls[0].clone();
    for _ in 0..3 {
        current = queue.next_track(&current).unwrap();
    }
    let next = queue.next_track(&current).unwrap();
    assert_ne!(
        next, current,
        "Shuffled rounds must not immediately repeat the last track"
    );
    let mut round = queue.entries();
    round.push(next);
    round.sort();
    assert_eq!(round, urls);
    queue.clear();
    assert!(!queue.has_previous());
}

#[test]
fn playlist_opens_compact_audio_with_titles_shuffle_and_repeat_controls() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let mut registry =
        PlaybackRegistry::with_audio_factory(tools(), factory(Arc::new(AtomicBool::new(false))));
    let mut windows = WindowManager::new();
    let mut atlas = ModAtlasPacker::new();
    let id = registry
        .play_playlist(
            vec![
                (server.url("/audio.wav"), "First song".into()),
                (server.url("/av.mp4"), "Second song".into()),
            ],
            false,
            true,
            &mut windows,
            &mut atlas,
        )
        .unwrap();
    let player = registry.get(id).unwrap();
    assert!(player.audio_only);
    let control = player.player.control.clone();
    wait(&control, |s| {
        s.repeat_all && s.queue_titles == ["Second song"]
    });
    assert!(control.snapshot().video_size.is_none());
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, true);
    assert_eq!(windows.window_mut(id).unwrap().rect.h, 31.);
    assert_eq!(windows.window_mut(id).unwrap().min_size, [200., 31.]);
    registry.get(id).unwrap().view.lock().queue_open = true;
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, true);
    assert_eq!(windows.window_mut(id).unwrap().rect.h, 115.);
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, false);
    assert_eq!(windows.window_mut(id).unwrap().rect.h, 31.);
    control.send(PlayerCommand::CycleRepeat).unwrap();
    wait(&control, |s| s.looping && !s.repeat_all);
    control.send(PlayerCommand::CycleRepeat).unwrap();
    wait(&control, |s| !s.looping && !s.repeat_all);
    control.send(PlayerCommand::SetShuffle(true)).unwrap();
    wait(&control, |s| s.shuffle);
}
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "lmp-m4-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&p).unwrap();
        Self(p)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Server {
    base: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(files: Vec<(String, Vec<u8>)>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let end = stop.clone();
        let files = Arc::new(files);
        let worker = thread::spawn(move || {
            let mut clients = Vec::new();
            while !end.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let files = files.clone();
                        clients.push(thread::spawn(move || serve(stream, &files)));
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(e) => panic!("{e}"),
                }
            }
            for client in clients {
                client.join().unwrap();
            }
        });
        Self {
            base,
            stop,
            worker: Some(worker),
        }
    }
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        self.worker.take().unwrap().join().unwrap();
    }
}
fn serve(mut stream: TcpStream, files: &[(String, Vec<u8>)]) {
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    let mut request = Vec::new();
    let mut b = [0; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 8192 {
        match stream.read(&mut b) {
            Ok(0) | Err(_) => return,
            Ok(n) => request.extend_from_slice(&b[..n]),
        }
    }
    let request = String::from_utf8_lossy(&request);
    let path = request
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap();
    let Some((_, bytes)) = files.iter().find(|(p, _)| p == path) else {
        let _ = stream
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let start = request
        .lines()
        .find_map(|line| {
            line.strip_prefix("Range: bytes=")
                .and_then(|s| s.split('-').next())
                .and_then(|n| n.parse::<usize>().ok())
        })
        .unwrap_or(0)
        .min(bytes.len());
    let header = format!(
        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
        bytes.len() - start,
        start,
        bytes.len() - 1,
        bytes.len()
    );
    if stream.write_all(header.as_bytes()).is_ok() {
        let _ = stream.write_all(&bytes[start..]);
    }
}
fn fixtures() -> Option<(Temp, Server)> {
    if Command::new("ffmpeg").arg("-version").output().is_err() {
        eprintln!("Skipping real decoder test: ffmpeg absent");
        return None;
    }
    let temp = Temp::new();
    let mut files = Vec::new();
    for (name, extra) in [
        (
            "av.mp4",
            vec![
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440:sample_rate=48000",
                "-c:a",
                "aac",
            ],
        ),
        ("silent.mp4", vec!["-an"]),
    ] {
        let mut command = Command::new("ffmpeg");
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=96x64:rate=30",
        ]);
        let result = command
            .args(extra)
            .args([
                "-t",
                "2",
                "-c:v",
                "mpeg4",
                "-threads",
                "1",
                "-movflags",
                "+faststart",
            ])
            .arg(temp.0.join(name))
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        files.push((format!("/{name}"), fs::read(temp.0.join(name)).unwrap()));
    }
    let result = Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=660:sample_rate=48000",
            "-t",
            "1",
            "-ac",
            "2",
        ])
        .arg(temp.0.join("audio.wav"))
        .output()
        .unwrap();
    assert!(result.status.success());
    files.push((
        "/audio.wav".into(),
        fs::read(temp.0.join("audio.wav")).unwrap(),
    ));
    let server = Server::new(files);
    Some((temp, server))
}
fn tools() -> ToolManager {
    let mut t = ToolManager::new(None);
    t.policy.auto_download = false;
    t
}
struct TestSink {
    renderer: AudioRenderer,
    previous: Instant,
    audible: Arc<AtomicBool>,
}
impl AudioSink for TestSink {
    fn poll(&mut self) {
        let now = Instant::now();
        let count = (now.duration_since(self.previous).as_secs_f64() * 48000.) as usize;
        self.previous = now;
        for _ in 0..count.min(9600) {
            if self.renderer.frame(48000)[0].abs() > 0.001 {
                self.audible.store(true, Ordering::Release);
            }
        }
    }
}
fn factory(audible: Arc<AtomicBool>) -> AudioFactory {
    Arc::new(move |pcm, clock| {
        Ok(Box::new(TestSink {
            renderer: AudioRenderer::new(pcm, clock),
            previous: Instant::now(),
            audible: audible.clone(),
        }))
    })
}
fn wait(
    control: &PlayerControl,
    mut predicate: impl FnMut(&PlayerSnapshot) -> bool,
) -> Arc<PlayerSnapshot> {
    let start = Instant::now();
    loop {
        let s = control.snapshot();
        assert_ne!(s.state, PlaybackState::Failed, "{:?}", s.error);
        if predicate(&s) {
            return s;
        }
        assert!(
            start.elapsed() < Duration::from_secs(12),
            "Timed out: {s:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn real_av_player_seek_pause_speed_queue_and_cleanup() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let audible = Arc::new(AtomicBool::new(false));
    let player = Player::with_audio_factory(tools(), false, factory(audible.clone()));
    let c = &player.control;
    c.send(PlayerCommand::Load(server.url("/av.mp4"))).unwrap();
    wait(c, |s| s.position_secs > 0.25 && s.video_size.is_some());
    assert!(audible.load(Ordering::Acquire));
    let frame = c.take_frame().unwrap();
    assert_eq!(frame.width, 512);
    assert!(frame.pts <= c.snapshot().position_secs + 0.05);
    c.send(PlayerCommand::Pause).unwrap();
    let paused = wait(c, |s| s.state == PlaybackState::Paused).position_secs;
    thread::sleep(Duration::from_millis(100));
    assert_eq!(c.snapshot().position_secs, paused);
    c.send(PlayerCommand::Seek(0.8)).unwrap();
    wait(c, |s| {
        s.state == PlaybackState::Paused && (s.position_secs - 0.8).abs() < 0.01
    });
    c.send(PlayerCommand::SetSpeed(2.)).unwrap();
    c.send(PlayerCommand::SetFps(60)).unwrap();
    c.send(PlayerCommand::SetTargetHeight(1080)).unwrap();
    c.send(PlayerCommand::Play).unwrap();
    wait(c, |s| {
        s.speed == 2. && s.fps == 60 && s.target_height == 1080 && s.position_secs > 1.0
    });
    c.send(PlayerCommand::Enqueue(server.url("/silent.mp4")))
        .unwrap();
    wait(c, |s| {
        s.source_url.ends_with("silent.mp4") && s.position_secs > 0.1
    });
    wait(c, |s| s.state == PlaybackState::Ended);
    let now = Instant::now();
    player.join();
    assert!(now.elapsed() < Duration::from_secs(2));
}
#[test]
fn real_audio_only_loops_and_can_suspend_without_output_device() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let audible = Arc::new(AtomicBool::new(false));
    let player = Player::with_audio_factory(tools(), true, factory(audible.clone()));
    let c = &player.control;
    c.send(PlayerCommand::Load(server.url("/audio.wav")))
        .unwrap();
    c.send(PlayerCommand::SetLoop(true)).unwrap();
    let first = wait(c, |s| s.position_secs > 0.15);
    wait(c, |s| {
        s.generation > first.generation && s.position_secs > 0.1
    });
    assert!(audible.load(Ordering::Acquire));
    assert!(c.snapshot().video_size.is_none());
    c.send(PlayerCommand::Suspend).unwrap();
    let paused = wait(c, |s| s.state == PlaybackState::Paused).position_secs;
    thread::sleep(Duration::from_millis(80));
    assert_eq!(c.snapshot().position_secs, paused);
    c.send(PlayerCommand::SetLoop(false)).unwrap();
    c.send(PlayerCommand::Play).unwrap();
    wait(c, |s| s.state == PlaybackState::Ended);
    player.join();
}
#[test]
fn split_stream_session_and_full_buffers_drop_promptly() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let cancel = Cancellation::default();
    let mut media = DirectResolver::resolve(&server.url("/silent.mp4"), MediaKind::Video).unwrap();
    media.audio = Some(StreamUrl {
        url: server.url("/audio.wav"),
        headers: BTreeMap::new(),
    });
    let probe = MediaProbe {
        video: true,
        audio: true,
        duration: Some(2.),
    };
    let (mut session, mut pcm) =
        FfmpegSession::start(&tools(), &media, &probe, SessionOptions::default(), &cancel).unwrap();
    let start = Instant::now();
    while session.frames.is_empty() || session.received_audio.load(Ordering::Acquire) == 0 {
        session.poll().unwrap();
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    assert!(pcm.as_mut().unwrap().pop().is_some());
    thread::sleep(Duration::from_millis(300));
    let start = Instant::now();
    drop(session);
    assert!(start.elapsed() < Duration::from_secs(2));
}
#[test]
fn registry_limits_hidden_mode_resize_unload_and_window_input() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let mut registry =
        PlaybackRegistry::with_audio_factory(tools(), factory(Arc::new(AtomicBool::new(false))));
    registry.max_concurrent_videos = 1;
    let mut windows = WindowManager::new();
    let mut atlas = ModAtlasPacker::new();
    let id = registry
        .request(
            &server.url("/av.mp4"),
            false,
            true,
            &mut windows,
            &mut atlas,
        )
        .unwrap();
    assert!(
        registry
            .request(
                &server.url("/silent.mp4"),
                false,
                true,
                &mut windows,
                &mut atlas
            )
            .is_err()
    );
    let control = registry.get(id).unwrap().player.control.clone();
    wait(&control, |s| {
        s.state == PlaybackState::Playing && s.video_size.is_some()
    });
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, true);
    assert!(registry.get(id).unwrap().view.lock().texture.is_some());
    windows.window_mut(id).unwrap().hidden = true;
    registry.tick(Duration::from_secs(6), &mut windows, &mut atlas, None, true);
    wait(&control, |s| {
        s.sound_only && s.state == PlaybackState::Playing
    });
    windows.reveal();
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, true);
    wait(&control, |s| {
        !s.sound_only && s.state == PlaybackState::Playing
    });
    windows.window_mut(id).unwrap().rect.h = 550.;
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, true);
    registry.tick(
        Duration::from_millis(600),
        &mut windows,
        &mut atlas,
        None,
        true,
    );
    wait(&control, |s| s.target_height == 720);
    registry.suspend(&mut atlas);
    wait(&control, |s| s.state == PlaybackState::Paused);
    windows.clear();
    registry.restore(&mut windows);
    assert_eq!(windows.windows().len(), 1);
    let view = Arc::new(Mutex::new(PlaybackView::new(control.clone())));
    let mut widget = VideoWindow::new(view);
    widget.layout(Rect::new(0., 0., 400., 240.));
    let mut input = InputState {
        focus: Some(widget.id()),
        capture: None,
    };
    let mut actions = Vec::new();
    assert!(widget.event(
        &InputEvent::Key {
            key: Key::Space,
            mods: Modifiers::default()
        },
        &mut EventContext {
            input: &mut input,
            actions: &mut actions
        }
    ));
    wait(&control, |s| s.state == PlaybackState::Playing);
    windows.close(id);
    registry.tick(Duration::ZERO, &mut windows, &mut atlas, None, true);
    assert!(registry.get(id).is_none());
}
#[test]
fn failures_are_visible_and_retryable() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let player = Player::with_audio_factory(
        tools(),
        true,
        Arc::new(|_, _| Err("No audio output device (test)".into())),
    );
    player
        .control
        .send(PlayerCommand::Load(server.url("/audio.wav")))
        .unwrap();
    let start = Instant::now();
    while player.control.snapshot().state != PlaybackState::Failed {
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        player
            .control
            .snapshot()
            .error
            .as_ref()
            .unwrap()
            .contains("No audio output device")
    );
    player.join();
}

#[test]
#[ignore = "opens the default OS audio output device and sends silent PCM"]
fn cpal_device_callback_consumes_pcm_and_honors_pause() {
    let (mut producer, consumer) = PcmRing::bounded(96_000);
    for _ in 0..48_000 {
        assert!(producer.push([0, 0]));
    }
    let clock = Arc::new(AudioClock::default());
    let _sink = CpalAudioSink::open(consumer, clock.clone()).expect("default audio output device");
    clock.pause(false);
    let start = Instant::now();
    while clock.seconds() < 0.05 {
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "device callback never consumed PCM"
        );
        thread::sleep(Duration::from_millis(10));
    }
    clock.pause(true);
    thread::sleep(Duration::from_millis(30));
    let frozen = clock.played_frames.load(Ordering::Acquire);
    thread::sleep(Duration::from_millis(60));
    assert_eq!(clock.played_frames.load(Ordering::Acquire), frozen);
    assert!(!clock.failed.load(Ordering::Acquire));
}

#[test]
fn unload_interrupts_inflight_network_probe_and_full_mailbox() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/slow.mp4", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let player =
        Player::with_audio_factory(tools(), false, factory(Arc::new(AtomicBool::new(false))));
    player.control.send(PlayerCommand::Load(url)).unwrap();
    let start = Instant::now();
    let connection = loop {
        if let Ok((s, _)) = listener.accept() {
            break s;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        thread::sleep(Duration::from_millis(5));
    };
    for _ in 0..100 {
        let _ = player.control.send(PlayerCommand::SetVolume(0.5));
    }
    let start = Instant::now();
    player.control.send(PlayerCommand::Suspend).unwrap();
    wait(&player.control, |s| s.state == PlaybackState::Paused);
    assert!(start.elapsed() < Duration::from_secs(2));
    drop(connection);
    player.join();
}

#[test]
fn short_audio_track_does_not_deadlock_the_video_tail() {
    let Some((_temp, server)) = fixtures() else {
        return;
    };
    let mut media = DirectResolver::resolve(&server.url("/silent.mp4"), MediaKind::Video).unwrap();
    media.audio = Some(StreamUrl {
        url: server.url("/audio.wav"),
        headers: BTreeMap::new(),
    });
    let probe = MediaProbe {
        video: true,
        audio: true,
        duration: Some(2.),
    };
    let (mut session, pcm) = FfmpegSession::start(
        &tools(),
        &media,
        &probe,
        SessionOptions::default(),
        &Cancellation::default(),
    )
    .unwrap();
    let clock = Arc::new(AudioClock::default());
    clock.pause(false);
    let mut renderer = AudioRenderer::new(pcm.unwrap(), clock.clone());
    let start = Instant::now();
    let mut last_pts = 0.;
    loop {
        for _ in 0..480 {
            renderer.frame(48000);
        }
        if let Some(frame) = session.frames.take_due(clock.seconds()) {
            last_pts = frame.pts;
        }
        if session.poll().unwrap() == Some(true) && session.frames.is_empty() && last_pts > 1.8 {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "Video tail stalled at PCM {} / video {last_pts}",
            clock.seconds()
        );
        thread::sleep(Duration::from_millis(2));
    }
    assert!(clock.seconds() > 1.8);
}
