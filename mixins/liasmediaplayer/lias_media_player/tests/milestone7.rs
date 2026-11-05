use lias_media_player::{
    api::*,
    core::{
        resolver::{DirectResolver, ResolverChain},
        sources::SourceRegistry,
        tools::ToolManager,
    },
    platform::*,
    runtime::with_runtime,
};
use std::{
    io::{self, Read, Write},
    net::TcpListener,
    sync::Mutex,
    time::{Duration, Instant},
};
static TEST: Mutex<()> = Mutex::new(());

struct Source;
impl LinkSource for Source {
    fn id(&self) -> &str {
        "m7:radio"
    }
    fn matches(&self, url: &Url) -> Option<(MediaKind, MediaKind, &str)> {
        (url.host_str() == Some("m7.test")).then_some((MediaKind::Audio, MediaKind::Audio, "radio"))
    }
}
struct Radio;
impl Resolver for Radio {
    fn id(&self) -> &str {
        "m7:radio"
    }
    fn resolve(
        &self,
        request: ResolveRequest<'_>,
        cancel: &Cancellation,
    ) -> Option<io::Result<ResolvedMedia>> {
        let url = Url::parse(request.url).ok()?;
        if url.host_str() != Some("m7.test") {
            return None;
        }
        let mut media =
            DirectResolver::resolve("https://cdn.test/radio.mp3", MediaKind::Audio).unwrap();
        if url.path() == "/cancel" {
            cancel.cancel();
        }
        if url.path() == "/file" {
            media.audio.as_mut().unwrap().url = "file:///etc/passwd".into();
        }
        if url.path() == "/header" {
            media
                .audio
                .as_mut()
                .unwrap()
                .headers
                .insert("X-Test".into(), "a\r\nb".into());
        }
        media.title = Some("Addon radio".into());
        Some(Ok(media))
    }
}
struct Addon;
impl MediaPlayerAddon for Addon {
    fn id(&self) -> &str {
        "m7"
    }
    fn register(&self, ctx: &mut AddonContext) -> Result<(), RegisterError> {
        ctx.source(Source);
        ctx.resolver(Radio);
        ctx.theme(ThemeSpec::new("m7:violet", "Violet").with(ThemeRole::Fill, 0xffaa00aa));
        ctx.window_action(WindowAction {
            id: "m7:star".into(),
            label: "*".into(),
        });
        Ok(())
    }
}
fn register() {
    let _ = register_addon(Box::new(Addon));
}

#[test]
fn live_registries_resolve_custom_sources_validate_results_and_cancel() {
    let _guard = TEST.lock().unwrap();
    let sources = SourceRegistry::default();
    let chain = ResolverChain::new(ToolManager::new(None));
    register();
    assert_eq!(
        sources.identify("https://m7.test/track.mp4").unwrap().kind,
        MediaKind::Audio
    );
    assert!(sources.identify("https://m7.test.evil/track").is_none());
    let media = chain
        .resolve(
            "https://m7.test/track",
            MediaKind::Video,
            ResolveOptions::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(media.source_url, "https://m7.test/track");
    assert_eq!(media.title.as_deref(), Some("Addon radio"));
    assert_eq!(media.audio.unwrap().url, "https://cdn.test/radio.mp3");
    for path in ["file", "header", "cancel"] {
        assert!(
            chain
                .resolve(
                    &format!("https://m7.test/{path}"),
                    MediaKind::Video,
                    ResolveOptions::default(),
                    &Cancellation::default()
                )
                .is_err()
        );
    }
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        chain
            .resolve(
                "https://m7.test/track",
                MediaKind::Audio,
                ResolveOptions::default(),
                &cancel
            )
            .is_err()
    );
    assert!(
        register_addon(Box::new(Addon))
            .unwrap_err()
            .0
            .contains("already registered")
    );
}

struct Bad {
    id: &'static str,
    mode: u8,
}
impl MediaPlayerAddon for Bad {
    fn id(&self) -> &str {
        self.id
    }
    fn api_version(&self) -> u32 {
        if self.mode == 4 { 999 } else { API_VERSION }
    }
    fn register(&self, ctx: &mut AddonContext) -> Result<(), RegisterError> {
        ctx.theme(ThemeSpec::new(format!("{}:valid", self.id), "Uncommitted"));
        match self.mode {
            0 => ctx.theme(ThemeSpec::new("someone_else:theme", "Invalid")),
            1 => return Err(RegisterError("registration failed".into())),
            2 => panic!("registration panic"),
            3 => ctx.theme(ThemeSpec::new(format!("{}:valid", self.id), "Duplicate")),
            _ => (),
        }
        Ok(())
    }
}
#[test]
fn failed_registration_is_atomic_and_does_not_reserve_namespace() {
    let _guard = TEST.lock().unwrap();
    for (mode, id) in [
        (0, "bad0"),
        (1, "bad1"),
        (2, "bad2"),
        (3, "bad3"),
        (4, "bad4"),
    ] {
        assert!(register_addon(Box::new(Bad { id, mode })).is_err());
        assert!(!themes().iter().any(|t| t.id.starts_with(id)));
        register_addon(Box::new(Bad { id, mode: 5 })).unwrap();
    }
    assert!(
        register_addon(Box::new(Bad {
            id: "liasmediaplayer",
            mode: 5
        }))
        .is_err()
    );
}

#[derive(Default)]
struct CanvasLog(Vec<String>);
impl Canvas for CanvasLog {
    fn size(&self) -> [f32; 2] {
        [800., 600.]
    }
    fn fill(&mut self, _: Rect, _: u32) {}
    fn texture(&mut self, _: TextureId, _: Rect, _: [f32; 4]) {}
    fn text(&mut self, text: &str, _: [f32; 2], _: u32, _: bool) {
        self.0.push(text.into());
    }
    fn text_width(&self, s: &str) -> f32 {
        s.chars().count() as f32 * 6.
    }
    fn push_clip(&mut self, _: Rect) {}
    fn pop_clip(&mut self) {}
}
#[test]
fn media_handles_events_actions_themes_and_world_lifecycle() {
    let _guard = TEST.lock().unwrap();
    register();
    let root = std::env::temp_dir().join(format!("lmp-m7-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("settings.json"),
        br#"{"theme":"m7:violet","autoDownloadTools":false}"#,
    )
    .unwrap();
    with_runtime(|r| {
        r.configure(&root, &root);
        assert_eq!(r.ui.windows.theme.id, "m7:violet");
        assert!(active_media().unwrap_err().contains("busy"));
    });
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/picture.png", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = [0; 4096];
        let _ = socket.read(&mut request);
        let png = include_bytes!("fixtures/rgba.png");
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            png.len()
        )
        .unwrap();
        socket.write_all(png).unwrap();
    });
    let events = subscribe(32).unwrap();
    let slow = subscribe(1).unwrap();
    let handle = open(&url, MediaKind::Image).unwrap();
    let end = Instant::now() + Duration::from_secs(5);
    loop {
        with_runtime(|r| r.on_frame(Duration::from_millis(20)));
        if handle.snapshot().unwrap().state == PlaybackState::Playing {
            break;
        }
        assert!(Instant::now() < end);
        std::thread::sleep(Duration::from_millis(10));
    }
    server.join().unwrap();
    assert_eq!(handle.snapshot().unwrap().source_url, url);
    handle.command(PlayerCommand::Pause).unwrap();
    handle.window(WindowCommand::Hide).unwrap();
    with_runtime(|r| r.on_frame(Duration::ZERO));
    assert!(handle.snapshot().unwrap().hidden);
    assert_eq!(handle.snapshot().unwrap().state, PlaybackState::Paused);
    handle.window(WindowCommand::Show).unwrap();
    with_runtime(|r| {
        let mut canvas = CanvasLog::default();
        r.ui.windows.draw(&mut canvas, true, None);
        assert!(canvas.0.iter().any(|s| s == "*"));
        let w =
            r.ui.windows
                .windows()
                .iter()
                .find(|w| w.id == handle.id())
                .unwrap();
        // Exact left edge of the custom button belongs to that button.
        let at = [w.rect.right() - 100., w.rect.y + 10.];
        assert!(r.input(&InputEvent::Down {
            at,
            button: 0,
            mods: Modifiers::default()
        }));
        r.input(&InputEvent::Up { at, button: 0 });
    });
    let observed: Vec<_> = events.try_iter().collect();
    assert!(
        observed
            .iter()
            .any(|e| matches!(e, MediaEvent::Opened(s) if s.handle == handle))
    );
    assert!(
        observed
            .iter()
            .any(|e| matches!(e, MediaEvent::Changed(s) if s.state == PlaybackState::Paused))
    );
    assert!(observed.iter().any(
        |e| matches!(e, MediaEvent::WindowAction {id, handle:h} if id == "m7:star" && *h == handle)
    ));
    slow.try_recv().unwrap();
    assert!(matches!(
        slow.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Disconnected)
    ));
    handle.window(WindowCommand::Close).unwrap();
    with_runtime(|r| r.on_frame(Duration::ZERO));
    assert!(handle.snapshot().is_err());
    assert!(handle.command(PlayerCommand::Play).is_err());
    assert!(
        events
            .try_iter()
            .any(|e| matches!(e, MediaEvent::Closed(h) if h == handle))
    );
    with_runtime(|r| r.on_world_unload());
    assert!(open(&url, MediaKind::Image).is_err());
    with_runtime(|r| {
        r.configure(&root, &root);
        r.on_disabled();
    });
    assert!(open(&url, MediaKind::Audio).is_err());
    with_runtime(|r| {
        r.on_world_unload();
        *r = Default::default();
    });
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn subscriptions_release_capacity_without_requiring_an_event() {
    let _guard = TEST.lock().unwrap();
    for _ in 0..100 {
        drop(subscribe(1).unwrap());
    }
    let subscriptions: Vec<_> = (0..64).map(|_| subscribe(1).unwrap()).collect();
    assert!(subscribe(1).is_err());
    drop(subscriptions);
    assert!(subscribe(1).is_ok());
}

#[path = "../../../../examples/addon/addon.rs"]
mod example;
#[test]
fn documented_addon_compiles_and_registers() {
    let _guard = TEST.lock().unwrap();
    example::install();
    example::install();
    assert_eq!(
        SourceRegistry::default()
            .identify("https://radio.example.org/listen")
            .unwrap()
            .kind,
        MediaKind::Audio
    );
}
