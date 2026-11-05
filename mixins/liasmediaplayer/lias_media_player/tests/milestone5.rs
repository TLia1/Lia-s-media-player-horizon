use lias_media_player::{
    Runtime,
    core::{chat::*, player::PlayQueue, previews::PreviewCache, sources::*, tools::ToolManager},
    platform::{horizon_impl::ModAtlasPacker, *},
    ui::chat_overlay::*,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
fn span(text: &str, style: u32) -> ChatSpan<u32> {
    ChatSpan {
        text: text.into(),
        style,
        url: None,
        protected: false,
    }
}
fn wait(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out");
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn unicode_multi_links_and_cross_style_urls_keep_surrounding_styles() {
    let spans = vec![
        span("é before https://youtu.", 1),
        span("be/abc — https://a.test/a.gif! after", 2),
    ];
    let out = rewrite(&spans, &SourceRegistry::default(), true);
    assert_eq!(
        out.iter().map(|s| s.text.as_str()).collect::<String>(),
        "é before [youtube] — [gif]! after"
    );
    assert_eq!(out[0].style, 1);
    assert_eq!(out[1].style, 1);
    assert_eq!(out.last().unwrap().style, 2);
    assert_eq!(out.iter().filter(|s| s.media.is_some()).count(), 2);
    assert_eq!(out[1].media.as_ref().unwrap().url, "https://youtu.be/abc");
    let raw = rewrite(&spans, &SourceRegistry::default(), false);
    assert_eq!(
        raw.iter().map(|s| s.text.as_str()).collect::<String>(),
        spans.iter().map(|s| s.text.as_str()).collect::<String>()
    );
}
#[test]
fn labelled_links_unknown_links_and_existing_actions_are_preserved() {
    let mut named = span("watch this", 7);
    named.url = Some("https://youtu.be/abc".into());
    let mut command = span("https://a.test/a.png", 9);
    command.protected = true;
    let spans = vec![named, span(" https://example.com/docs ", 3), command];
    let out = rewrite(&spans, &SourceRegistry::default(), true);
    assert_eq!(out[0].text, "[youtube]");
    assert_eq!(out[0].style, 7);
    assert!(
        out.iter()
            .any(|s| s.text.contains("example.com/docs") && s.media.is_none())
    );
    assert_eq!(out.last().unwrap().text, "https://a.test/a.png");
    assert!(out.last().unwrap().media.is_none());
    let mut mismatch = span("https://a.test/good.png", 0);
    mismatch.url = Some("https://a.test/actual.mp4".into());
    assert_eq!(
        rewrite(&[mismatch], &SourceRegistry::default(), true)[0]
            .media
            .as_ref()
            .unwrap()
            .url,
        "https://a.test/actual.mp4"
    );
}
#[test]
fn modifiers_priority_and_non_media_passthrough() {
    let r = SourceRegistry::default();
    let u = "https://a.test/a.mp4";
    assert!(matches!(
        click_action(u, Modifiers::default(), &r),
        Some(ChatAction::Open(_))
    ));
    assert!(matches!(
        click_action(u, Modifiers::from_bits(1), &r),
        Some(ChatAction::Enqueue(_))
    ));
    assert_eq!(
        click_action(u, Modifiers::from_bits(3), &r),
        Some(ChatAction::Browser(u.into()))
    );
    for u in [
        "file:///a.mp4",
        "https://youtube.com.evil/watch?v=x",
        "https://example.com",
    ] {
        assert!(click_action(u, Modifiers::default(), &r).is_none());
    }
}
#[test]
fn chip_hitboxes_hidden_reveal_and_card_clamping() {
    let size = [640., 360.];
    let chips = ChatOverlay::chips(size, 2);
    assert_eq!(chips.len(), 2);
    for (r, _, action) in chips {
        assert_eq!(
            ChatOverlay::chip_click(size, 2, [r.x + 2., r.y + 2.]),
            Some(action)
        );
    }
    assert!(ChatOverlay::chip_click(size, 2, [10., 359.]).is_none());
    assert_eq!(ChatOverlay::chips(size, 0).len(), 1);
    for size in [[640., 360.], [140., 100.]] {
        let r = ChatOverlay::card_rect(size, [size[0] - 1., size[1] - 1.], 190.);
        assert!(r.x >= 0. && r.y >= 0. && r.right() <= size[0] && r.bottom() <= size[1]);
    }
    let mut rt = Runtime::new();
    rt.ui.open();
    let id = rt.ui.windows.windows()[0].id;
    rt.ui.windows.window_mut(id).unwrap().hidden = true;
    rt.chat_action(ChatAction::RevealHidden);
    assert!(!rt.ui.windows.windows()[0].hidden);
}
#[test]
fn atomic_collection_queue_limits() {
    let mut q = PlayQueue::default();
    q.enqueue_batch(
        (0..499)
            .map(|i| format!("https://a.test/{i}.mp3"))
            .collect(),
    )
    .unwrap();
    assert!(
        q.enqueue_batch(vec![
            "https://a.test/a.mp3".into(),
            "https://a.test/b.mp3".into()
        ])
        .is_err()
    );
    assert_eq!(q.entries().len(), 499);
    assert!(q.enqueue_batch(vec!["file:///bad".into()]).is_err());
    assert_eq!(q.entries().len(), 499);
    q.enqueue_batch(vec!["https://a.test/final.mp3".into()])
        .unwrap();
    assert_eq!(q.entries().len(), 500);
}
#[test]
fn java_urls_and_rust_playlist_entries_and_empty_library() {
    let lib = serde_json::json!({"playlists":[{"name":"Java","urls":["https://a.test/a.mp3"]},{"name":"Rust","entries":[{"url":"https://a.test/a.png","title":"Picture"}]}]});
    let rows = playlist_rows(&lib, &SourceRegistry::default());
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[1].media.as_ref().unwrap().kind, MediaKind::Audio);
    assert_eq!(rows[3].label, "Picture");
    assert!(playlist_rows(&serde_json::json!({}), &SourceRegistry::default()).is_empty());
    let mut rt = Runtime::new();
    rt.chat_action(ChatAction::Playlists);
    assert!(rt.open_playlist_screen);
    assert_eq!(
        rt.screens.active,
        Some(lias_media_player::ui::screens::ScreenKind::Playlists)
    );
    rt.on_world_unload();
    assert!(rt.chat.picker.is_none());
    assert!(!rt.open_playlist_screen);
}
fn server(
    status: &str,
    bytes: &'static [u8],
) -> (String, Arc<AtomicUsize>, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/a.png", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let seen = count.clone();
    let status = status.to_owned();
    let worker = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(4);
        while Instant::now() < until {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    stream
                        .set_read_timeout(Some(Duration::from_secs(1)))
                        .unwrap();
                    let mut buf = [0; 2048];
                    let _ = stream.read(&mut buf);
                    seen.fetch_add(1, Ordering::SeqCst);
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        bytes.len()
                    );
                    let _ = stream.write_all(bytes);
                    return;
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(5))
                }
                Err(e) => panic!("{e}"),
            }
        }
    });
    (url, count, worker)
}
#[test]
fn preview_is_async_cached_bounded_to_one_frame_and_restores_from_disk() {
    let (url, count, worker) = server("200 OK", include_bytes!("fixtures/disposal.gif"));
    let root = std::env::temp_dir().join(format!("lmp-m5-preview-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let media = SourceRegistry::default().identify(&url).unwrap();
    let mut cache = PreviewCache::new(Some(&root), ToolManager::new(None));
    let start = Instant::now();
    cache.request(&media);
    assert!(start.elapsed() < Duration::from_millis(100));
    wait(|| cache.get(&url).is_some());
    let p = cache.get(&url).unwrap();
    assert!(p.error.is_none());
    let image = p.image.as_ref().unwrap();
    assert_eq!(image.frames.len(), 1);
    assert!(image.width <= 192 && image.height <= 192);
    cache.request(&media);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    worker.join().unwrap();
    drop(cache);
    let mut restored = PreviewCache::new(Some(&root), ToolManager::new(None));
    restored.request(&media);
    wait(|| restored.get(&url).is_some());
    assert!(restored.get(&url).unwrap().image.is_some());
    drop(restored);
    // Preview workers shut down off-thread; titles.json may flush while cleanup runs.
    wait(|| match std::fs::remove_dir_all(&root) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => true,
        Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => false,
        Err(e) => panic!("preview cleanup: {e}"),
    });
}
#[test]
fn failed_preview_is_cached_and_clearing_hover_releases_atlas() {
    let (url, count, worker) = server("404 Not Found", b"no image");
    let media = SourceRegistry::default().identify(&url).unwrap();
    let mut cache = PreviewCache::new(None, ToolManager::new(None));
    cache.request(&media);
    wait(|| cache.get(&url).is_some());
    assert!(cache.get(&url).unwrap().error.is_some());
    cache.cancel();
    cache.request(&media);
    assert_eq!(count.load(Ordering::SeqCst), 1);
    worker.join().unwrap();
    let mut overlay = ChatOverlay::default();
    let mut atlas = ModAtlasPacker::new();
    overlay.hover(Some(media), Some(&mut cache), &mut atlas);
    overlay.clear_hover(Some(&mut cache), &mut atlas);
    overlay.tick(Some(&mut cache), &mut atlas);
}

#[test]
fn fast_scan_handles_arbitrarily_split_scheme_and_settings_aliases() {
    assert!(has_http(["hi h", "T", "TpS://a.test/a.png"]));
    assert!(!has_http(["hello", "ordinary chat"]));
    let root = std::env::temp_dir().join(format!("lmp-m5-settings-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("settings.json"),r#"{"schema":3,"replaceChatLinks":false,"values":{"liasmediaplayer:chat.hover_preview":false}}"#).unwrap();
    let mut rt = Runtime::new();
    rt.configure(&root, &root.join("java"));
    assert!(!rt.replace_chat_links(|| None));
    assert!(!rt.chat_hover_previews);
    drop(rt);
    std::fs::remove_dir_all(root).unwrap();
}
#[derive(Default)]
struct CanvasLog {
    textures: Vec<TextureId>,
    texts: Vec<String>,
}
impl Canvas for CanvasLog {
    fn size(&self) -> [f32; 2] {
        [640., 360.]
    }
    fn fill(&mut self, _: Rect, _: u32) {}
    fn texture(&mut self, id: TextureId, _: Rect, _: [f32; 4]) {
        self.textures.push(id);
    }
    fn text(&mut self, text: &str, _: [f32; 2], _: u32, _: bool) {
        self.texts.push(text.into());
    }
    fn text_width(&self, text: &str) -> f32 {
        text.chars().count() as f32 * 6.
    }
    fn push_clip(&mut self, _: Rect) {}
    fn pop_clip(&mut self) {}
}
#[test]
fn preview_texture_is_released_and_picker_routes_saved_extensionless_entries() {
    let (url, _, worker) = server("200 OK", include_bytes!("fixtures/rgba.png"));
    let media = SourceRegistry::default().identify(&url).unwrap();
    let mut cache = PreviewCache::new(None, ToolManager::new(None));
    cache.request(&media);
    wait(|| cache.get(&url).is_some());
    worker.join().unwrap();
    let mut atlas = ModAtlasPacker::new();
    let mut overlay = ChatOverlay::default();
    overlay.hover(Some(media), Some(&mut cache), &mut atlas);
    thread::sleep(Duration::from_millis(260));
    overlay.tick(Some(&mut cache), &mut atlas);
    let mut canvas = CanvasLog::default();
    overlay.draw_chat(&mut canvas, [300., 300.], 0, Some(&cache));
    assert_eq!(canvas.textures.len(), 1);
    let texture = canvas.textures[0];
    assert!(atlas.region(texture).is_some());
    overlay.clear_hover(Some(&mut cache), &mut atlas);
    assert!(atlas.region(texture).is_none());
    let sources = SourceRegistry::default();
    let rows = playlist_rows(
        &serde_json::json!({"playlists":[{"name":"Saved","entries":[{"url":"https://a.test/track","kind":"audio","title":"Track"}]}]}),
        &sources,
    );
    overlay.show_picker("Playlists", rows);
    overlay.draw_picker(&mut canvas);
    let (used, action) = overlay.picker_input(
        &InputEvent::Down {
            at: [30., 75.],
            button: 0,
            mods: Modifiers::from_bits(1),
        },
        &sources,
    );
    assert!(used);
    assert!(matches!(action,Some(ChatAction::Enqueue(m))if m.url=="https://a.test/track"));
}
#[test]
fn canceled_preview_cannot_publish_stale_completion() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/slow.png", listener.local_addr().unwrap());
    let (started, received) = std::sync::mpsc::channel();
    let (release, finish) = std::sync::mpsc::channel();
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = [0; 2048];
        assert!(stream.read(&mut request).unwrap() > 0);
        started.send(()).unwrap();
        finish.recv_timeout(Duration::from_secs(3)).unwrap();
        let bytes = include_bytes!("fixtures/rgba.png");
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        );
        let _ = stream.write_all(bytes);
    });
    let mut cache = PreviewCache::new(None, ToolManager::new(None));
    cache.request(&SourceRegistry::default().identify(&url).unwrap());
    received.recv_timeout(Duration::from_secs(3)).unwrap();
    cache.cancel();
    // A subsequent collection preview provides a deterministic barrier after the cancelled job.
    let next = SourceRegistry::default()
        .identify("https://youtube.com/playlist?list=test")
        .unwrap();
    cache.request(&next);
    release.send(()).unwrap();
    wait(|| cache.get(&next.url).is_some());
    assert!(cache.get(&url).is_none());
    worker.join().unwrap();
}
