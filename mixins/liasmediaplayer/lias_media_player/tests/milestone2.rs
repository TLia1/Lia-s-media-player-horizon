use lias_media_player::{
    Runtime,
    core::{
        cache::{AsyncCache, DiskCache},
        image::{ImageLoader, MAX_DOWNLOAD},
        scanner::LinkScanner,
        sources::{MediaKind, SourceRegistry},
        store::{JsonStore, Stores, migrate},
    },
    platform::{horizon_impl::*, *},
    ui::{
        image_window::{ImageView, ImageWindow},
        *,
    },
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!(
            "lmp-m2-{}-{}",
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
const PNG: &[u8] = include_bytes!("fixtures/rgba.png");
const GIF: &[u8] = include_bytes!("fixtures/disposal.gif");
fn wait(mut predicate: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < until, "timed out");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn scanner_offsets_punctuation_and_registry_priorities() {
    let text = "é (HtTpS://example.org/a_(b)). HTTPS://example.org/x.PNG?x=1!";
    let spans = LinkScanner::scan(text);
    assert_eq!(spans.len(), 2);
    assert_eq!(spans[0].url, "HtTpS://example.org/a_(b)");
    for s in &spans {
        assert_eq!(&text[s.start..s.end], s.url);
    }
    let r = SourceRegistry::default();
    assert_eq!(r.sources().len(), 16);
    for (url, id, kind) in [
        ("https://a.org/A.WeBp?sig=x", "picture", MediaKind::Image),
        ("https://tenor.com/fr/view/a-123", "tenor", MediaKind::Image),
        (
            "https://giphy.com/gifs/cat-ABCdef123",
            "giphy",
            MediaKind::Image,
        ),
        (
            "https://youtube.com/playlist?list=abc",
            "youtube_playlist",
            MediaKind::Collection,
        ),
        (
            "https://youtube.com/watch?v=abc&list=def",
            "youtube_playlist",
            MediaKind::Collection,
        ),
        ("https://twitch.tv/name", "twitch", MediaKind::Video),
        (
            "https://player.vimeo.com/video/123",
            "vimeo",
            MediaKind::Video,
        ),
        (
            "https://streamable.com/e/abc123",
            "streamable",
            MediaKind::Video,
        ),
        (
            "https://old.reddit.com/r/a/comments/id/x",
            "reddit",
            MediaKind::Video,
        ),
        (
            "https://soundcloud.com/artist/sets/album",
            "soundcloud_set",
            MediaKind::Collection,
        ),
        ("https://snd.sc/abc", "soundcloud", MediaKind::Audio),
        (
            "https://artist.bandcamp.com/album/name",
            "bandcamp_album",
            MediaKind::Collection,
        ),
        (
            "https://artist.bandcamp.com/track/name",
            "bandcamp",
            MediaKind::Audio,
        ),
        ("https://a.org/x.MP4?q=1", "video_file", MediaKind::Video),
        ("https://a.org/x.opus", "audio_file", MediaKind::Audio),
        ("https://a.org/x.m3u8", "stream", MediaKind::Video),
    ] {
        let found = r.identify(url).unwrap();
        assert_eq!(found.source_id, format!("liasmediaplayer:{id}"));
        assert_eq!(found.kind, kind);
    }
    for bad in [
        "file:///x.png",
        "https://youtube.com.evil/watch?v=x",
        "https://vimeo.com/channels/1",
        "https://user:pass@a.org/a.png",
        "https://a.org/\nx.png",
    ] {
        assert!(r.identify(bad).is_none(), "{bad}");
    }
}
#[test]
fn decoders_match_independent_pixel_fixtures() {
    for (bytes, expected, w, h, n) in [
        (PNG, include_bytes!("fixtures/rgba.raw").as_slice(), 3, 2, 1),
        (
            include_bytes!("fixtures/rgba.webp").as_slice(),
            include_bytes!("fixtures/rgba.raw").as_slice(),
            3,
            2,
            1,
        ),
        (
            GIF,
            include_bytes!("fixtures/disposal.gif.raw").as_slice(),
            8,
            6,
            4,
        ),
        (
            include_bytes!("fixtures/interlaced.gif").as_slice(),
            include_bytes!("fixtures/interlaced.gif.raw").as_slice(),
            64,
            48,
            1,
        ),
        (
            include_bytes!("fixtures/animated.webp").as_slice(),
            include_bytes!("fixtures/animated.webp.raw").as_slice(),
            8,
            6,
            4,
        ),
    ] {
        let image = ImageLoader::decode(bytes).unwrap();
        assert_eq!((image.width, image.height, image.frames.len()), (w, h, n));
        let got: Vec<u8> = image.frames.iter().flat_map(|f| f.rgba.clone()).collect();
        assert_eq!(got.len(), expected.len());
        // Invisible RGB values are immaterial (WebP canonicalizes transparent pixels).
        for (g, e) in got
            .as_chunks::<4>()
            .0
            .iter()
            .zip(expected.as_chunks::<4>().0)
        {
            assert_eq!(
                g[3],
                e[3],
                "image {w}x{h}, frames {n}, signature {:?}",
                &bytes[..12]
            );
            if e[3] != 0 {
                if bytes.starts_with(b"RIFF") {
                    // image-webp uses fixed-point alpha blending; libwebp rounds differently.
                    assert!(g[..3].iter().zip(&e[..3]).all(|(a, b)| a.abs_diff(*b) <= 1));
                } else {
                    assert_eq!(g, e);
                }
            }
        }
    }
    let image = ImageLoader::decode(GIF).unwrap();
    assert_eq!(
        image.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(),
        vec![40, 60, 80, 100]
    );
    let bmp = ImageLoader::decode(include_bytes!("fixtures/rgb.bmp")).unwrap();
    assert_eq!(&bmp.frames[0].rgba[..4], &[255, 0, 0, 255]);
    assert_eq!(&bmp.frames[0].rgba[12..16], &[12, 34, 56, 255]);
    for jpeg in [
        include_bytes!("fixtures/rgb.jpg").as_slice(),
        include_bytes!("fixtures/gray.jpg").as_slice(),
    ] {
        let image = ImageLoader::decode(jpeg).unwrap();
        assert_eq!((image.width, image.height, image.weight()), (3, 2, 24));
    }
}
#[test]
fn image_limits_and_truncation_fail_without_panicking() {
    assert!(ImageLoader::decode(&vec![0; MAX_DOWNLOAD + 1]).is_err());
    let mut huge = GIF.to_vec();
    huge[6..10].copy_from_slice(&[255, 255, 255, 255]);
    assert!(ImageLoader::decode(&huge).is_err());
    for bytes in [
        PNG,
        GIF,
        include_bytes!("fixtures/rgb.bmp").as_slice(),
        include_bytes!("fixtures/rgba.webp").as_slice(),
    ] {
        for end in 0..bytes.len().min(40) {
            assert!(ImageLoader::decode(&bytes[..end]).is_err());
        }
    }
    let small = ImageLoader::decode(GIF).unwrap().fit(4);
    assert_eq!((small.width, small.height), (4, 3));
    assert!(small.frames.iter().all(|f| f.rgba.len() == 48));
}
#[test]
fn stores_debounce_preserve_unknowns_migrate_and_quarantine() {
    let t = Temp::new();
    let file = t.0.join("settings.json");
    fs::write(
        &file,
        br#"{"schemaVersion":2,"theme":"light","addon":{"x":42}}"#,
    )
    .unwrap();
    let mut s = JsonStore::open(file.clone(), json!({}), migrate).unwrap();
    assert_eq!(s.get()["schemaVersion"], 3);
    s.set(json!({"schema":3,"theme":"dark","addon":{"x":42}}))
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    s.set(json!({"schema":3,"theme":"contrast","addon":{"x":42}}))
        .unwrap();
    thread::sleep(Duration::from_millis(300));
    assert!(fs::read_to_string(&file).unwrap().contains("light"));
    wait(|| fs::read_to_string(&file).unwrap().contains("contrast"));
    assert_eq!(s.get()["addon"]["x"], 42);
    drop(s);
    fs::write(&file, b"broken {").unwrap();
    let s = JsonStore::open(file.clone(), json!({"theme":"dark"}), migrate).unwrap();
    s.flush().unwrap();
    assert!(
        fs::read_dir(&t.0).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains("broken-"))
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(file).unwrap()).unwrap()["theme"],
        "dark"
    );
}
#[test]
fn first_run_import_and_history_keep_user_data() {
    let t = Temp::new();
    let java = t.0.join("java");
    let root = t.0.join("native");
    fs::create_dir_all(&java).unwrap();
    fs::write(
        java.join("config.json"),
        br#"{"max_image_cache_mb":64,"addon:key":"kept"}"#,
    )
    .unwrap();
    fs::write(
        java.join("playlists.json"),
        br#"[{"id":"p","urls":["https://a.org/a.png"]}]"#,
    )
    .unwrap();
    fs::write(
        java.join("history.json"),
        br#"[{"url":"https://a.org/a.png","favorite":true}]"#,
    )
    .unwrap();
    fs::write(
        java.join("windows.json"),
        br#"{"image":{"x":20,"width":300}}"#,
    )
    .unwrap();
    {
        let mut s = Stores::open(&root, &java).unwrap();
        assert_eq!(s.settings.get()["memoryCacheSizeMb"], 64);
        assert_eq!(s.settings.get()["addon:key"], "kept");
        assert_eq!(s.windows.get()["kinds"]["image"]["width"], 300);
        s.record_image("https://a.org/a.png").unwrap();
        assert_eq!(s.library.get()["history"][0]["favorite"], true);
        s.flush().unwrap();
    }
    assert!(java.join("config.json").exists());
    fs::write(java.join("settings.json"), br#"{"theme":"wrong"}"#).unwrap();
    let s = Stores::open(&root, &java).unwrap();
    assert_ne!(s.settings.get()["theme"], "wrong");
    assert_eq!(s.library.get()["playlists"][0]["id"], "p");
}
#[test]
fn weighted_lru_single_flight_and_disk_budget() {
    let c = Arc::new(AsyncCache::new(10));
    c.insert("a", Arc::new(1), 4);
    c.insert("b", Arc::new(2), 4);
    assert_eq!(*c.get(&"a").unwrap(), 1);
    c.insert("c", Arc::new(3), 4);
    assert!(c.get(&"b").is_none());
    assert_eq!(c.weight(), 8);
    let loads = Arc::new(AtomicUsize::new(0));
    let barrier = Arc::new(std::sync::Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let c = c.clone();
            let loads = loads.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                c.get_or_load("same", || {
                    loads.fetch_add(1, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(30));
                    Ok::<_, ()>((9, 3))
                })
                .unwrap()
            })
        })
        .collect();
    for h in handles {
        assert_eq!(*h.join().unwrap(), 9);
    }
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    let t = Temp::new();
    {
        let mut d = DiskCache::open(&t.0, 6).unwrap();
        d.put("https://x/../../a", &[1; 4]).unwrap();
        d.put("b", &[2; 4]).unwrap();
        assert!(d.get("https://x/../../a").unwrap().is_none());
        assert_eq!(d.get("b").unwrap().unwrap(), [2; 4]);
        d.set_title("b", "A title").unwrap();
    }
    let d = DiskCache::open(&t.0, 6).unwrap();
    assert_eq!(d.title("b"), Some("A title"));
    assert_eq!(DiskCache::key("b").len(), 64);
}
struct Backend {
    textures: usize,
    texts: Vec<String>,
}
impl HudBackend for Backend {
    fn size(&self) -> [f32; 2] {
        [640., 480.]
    }
    fn fill(&mut self, _: Rect, _: u32) {}
    fn sprite(&mut self, _: [f32; 4], _: Rect) {
        self.textures += 1;
    }
    fn text(&mut self, t: &str, _: [f32; 2], _: u32, _: bool) {
        self.texts.push(t.into());
    }
    fn text_width(&self, t: &str) -> f32 {
        t.len() as f32 * 6.
    }
    fn vertex_count(&self) -> usize {
        0
    }
    fn clip_since(&mut self, _: usize, _: Rect) {}
}
fn draw(w: &mut WindowManager, a: &ModAtlasPacker, interactive: bool) {
    let mut canvas = HorizonCanvas::new(
        Backend {
            textures: 0,
            texts: vec![],
        },
        a,
    );
    w.draw(&mut canvas, interactive, Some([100., 100.]));
}
#[test]
fn image_pan_zoom_pause_step_and_cancel_use_window_input() {
    let image = Arc::new(ImageLoader::decode(GIF).unwrap());
    let mut a = ModAtlasPacker::new();
    let texture = a
        .allocate(image.width, image.height, &image.frames[0].rgba)
        .unwrap();
    let view = Arc::new(parking_lot::Mutex::new(ImageView::default()));
    {
        let mut v = view.lock();
        v.image = Some(image);
        v.texture = Some(texture);
    }
    let mut windows = WindowManager::default();
    windows.open(FloatingWindow::new(
        "GIF",
        Rect::new(20., 20., 300., 220.),
        Box::new(ImageWindow::new(view.clone())),
    ));
    draw(&mut windows, &a, true);
    let before = windows.windows()[0].rect;
    assert!(windows.event(&InputEvent::Scroll {
        at: [100., 100.],
        delta: 2.,
        mods: Modifiers {
            ctrl: true,
            ..Default::default()
        }
    }));
    assert!(view.lock().zoom > 1.);
    assert_eq!(windows.windows()[0].rect, before);
    windows.event(&InputEvent::Down {
        at: [100., 100.],
        button: 0,
        mods: Modifiers::default(),
    });
    let pan = view.lock().pan;
    windows.event(&InputEvent::Drag {
        at: [120., 130.],
        button: 0,
        mods: Modifiers::default(),
    });
    assert!((view.lock().pan[0] - pan[0] - 20.).abs() < 0.001);
    assert!((view.lock().pan[1] - pan[1] - 30.).abs() < 0.001);
    windows.event(&InputEvent::Cancel);
    let pan = view.lock().pan;
    windows.event(&InputEvent::Drag {
        at: [200., 200.],
        button: 0,
        mods: Modifiers::default(),
    });
    assert_eq!(view.lock().pan, pan);
    windows.event(&InputEvent::Down {
        at: [100., 100.],
        button: 0,
        mods: Modifiers::default(),
    });
    windows.event(&InputEvent::Up {
        at: [100., 100.],
        button: 0,
    });
    windows.event(&InputEvent::Key {
        key: Key::Right,
        mods: Modifiers::default(),
    });
    assert!(view.lock().paused);
    assert_eq!(view.lock().frame, 1);
    view.lock().advance(Duration::from_secs(1));
    assert_eq!(view.lock().frame, 1);
    windows.event(&InputEvent::Key {
        key: Key::Space,
        mods: Modifiers::default(),
    });
    view.lock().advance(Duration::from_millis(61));
    assert_eq!(view.lock().frame, 2);
    windows.event(&InputEvent::Key {
        key: Key::Home,
        mods: Modifiers::default(),
    });
    assert_eq!(view.lock().zoom, 1.);
    assert_eq!(view.lock().pan, [0., 0.]);
    draw(&mut windows, &a, false);
}
fn server(body: &'static [u8], status: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buf = [0; 4096];
        let _ = stream.read(&mut buf);
        write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        let _ = stream.write_all(body);
    });
    format!("http://{addr}/image")
}
#[test]
fn show_downloads_uploads_reuses_closes_and_restores_pins() {
    let t = Temp::new();
    let url = server(GIF, "200 OK");
    let mut rt = Runtime::new();
    rt.configure(&t.0, &t.0.join("java"));
    assert!(
        rt.execute_show(
            lias_media_player::command::parse(&format!("show gif {url}"))
                .unwrap()
                .unwrap()
        )
        .contains("Loading gif")
    );
    let id = rt.ui.windows.windows()[0].id;
    let view = rt.images.as_ref().unwrap().view(id).unwrap();
    wait(|| {
        rt.on_frame(Duration::from_millis(5));
        view.lock().image.is_some()
    });
    let texture = view.lock().texture.unwrap();
    assert!(rt.ui.atlas.region(texture).is_some());
    assert_eq!(
        rt.stores.as_ref().unwrap().library.get()["history"][0]["url"],
        url
    );
    rt.on_world_unload();
    rt.on_frame(Duration::from_secs(1));
    assert_eq!(
        rt.stores.as_ref().unwrap().windows.get()["pinned"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    rt.configure(&t.0, &t.0.join("java"));
    assert_eq!(rt.ui.windows.windows().len(), 1);
    let id = rt.ui.windows.windows()[0].id;
    let view = rt.images.as_ref().unwrap().view(id).unwrap();
    wait(|| {
        rt.on_frame(Duration::from_millis(5));
        view.lock().image.is_some()
    });
    // Restored image is served from the cache: the one-request HTTP server is gone.
    let second = server(PNG, "200 OK");
    rt.execute_show(
        lias_media_player::command::parse(&format!("show image {second}"))
            .unwrap()
            .unwrap(),
    );
    assert_eq!(rt.ui.windows.windows().len(), 1);
    assert_eq!(rt.ui.windows.windows()[0].id, id);
    let view = rt.images.as_ref().unwrap().view(id).unwrap();
    wait(|| {
        rt.on_frame(Duration::from_millis(5));
        view.lock().image.is_some()
    });
    let texture = view.lock().texture.unwrap();
    rt.ui.windows.close(id);
    rt.on_frame(Duration::ZERO);
    assert!(rt.ui.atlas.region(texture).is_none());
    assert!(
        rt.stores.as_ref().unwrap().windows.get()["pinned"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}
#[test]
fn http_errors_are_visible_and_closed_requests_stay_closed() {
    let url = server(b"not found", "404 Not Found");
    let mut rt = Runtime::new();
    rt.execute_show(
        lias_media_player::command::parse(&format!("show image {url}"))
            .unwrap()
            .unwrap(),
    );
    let id = rt.ui.windows.windows()[0].id;
    let view = rt.images.as_ref().unwrap().view(id).unwrap();
    wait(|| {
        rt.on_frame(Duration::ZERO);
        view.lock().status.contains("404")
    });
    assert!(view.lock().texture.is_none());
    let url = server(PNG, "200 OK");
    rt.execute_show(
        lias_media_player::command::parse(&format!("show image {url} true"))
            .unwrap()
            .unwrap(),
    );
    let id = rt.ui.windows.windows().last().unwrap().id;
    rt.ui.windows.close(id);
    for _ in 0..20 {
        rt.on_frame(Duration::ZERO);
        thread::sleep(Duration::from_millis(5));
    }
    assert!(rt.images.as_ref().unwrap().view(id).is_none());
    assert_eq!(rt.ui.windows.windows().len(), 1);
}

#[test]
fn failed_atomic_write_can_retry_and_future_fields_survive() {
    let t = Temp::new();
    let dir = t.0.join("data");
    fs::create_dir(&dir).unwrap();
    let file = dir.join("settings.json");
    let mut store =
        JsonStore::open(file.clone(), json!({"schema":7,"future":42}), migrate).unwrap();
    store.flush().unwrap();
    fs::remove_file(&file).unwrap();
    fs::remove_dir(&dir).unwrap();
    fs::write(&dir, b"block").unwrap();
    store.set(json!({"schema":7,"future":43})).unwrap();
    assert!(store.flush().is_err());
    fs::remove_file(&dir).unwrap();
    fs::create_dir(&dir).unwrap();
    store.flush().unwrap();
    let value: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
    assert_eq!(value["future"], 43);
    assert_eq!(value["schema"], 7);
}
#[test]
fn gif_frame_limit_is_enforced() {
    let mut bytes = b"GIF89a\x01\x00\x01\x00\x80\x00\x00\xff\x00\x00\x00\x00\x00".to_vec();
    let frame = b"\x2c\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02\x44\x01\x00";
    for _ in 0..257 {
        bytes.extend_from_slice(frame);
    }
    bytes.push(0x3b);
    assert!(
        ImageLoader::decode(&bytes)
            .unwrap_err()
            .to_string()
            .contains("256-frame")
    );
}
#[test]
fn namespaced_java_settings_migrate_and_select_theme() {
    let t = Temp::new();
    fs::write(t.0.join("config.json"), br#"{"liasmediaplayer:theme":"light","liasmediaplayer:max_image_cache_mb":64,"liasmediaplayer:blocked_domains":"a.com, b.org","addon:keep":42}"#).unwrap();
    let mut rt = Runtime::new();
    rt.configure(&t.0, &t.0);
    assert_eq!(rt.ui.windows.theme.id, "liasmediaplayer:light");
    let s = rt.stores.as_ref().unwrap().settings.get();
    assert_eq!(
        s["values"]["liasmediaplayer:performance.image_cache_mb"],
        64
    );
    assert_eq!(
        s["values"]["liasmediaplayer:chat.blocked_hosts"],
        json!(["a.com", "b.org"])
    );
    assert_eq!(s["values"]["addon:keep"], 42);
}
