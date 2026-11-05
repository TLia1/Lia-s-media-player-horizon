use lias_media_player::core::{
    image::{ImageLoader, ffmpeg_image::FfmpegImageDecoder},
    process::{Cancellation, ManagedProcess, ProcessRunner},
    resolver::*,
    sources::{MediaKind, SourceRegistry},
    tools::{Tool, ToolManager, allowed_download_url, executable_magic, extract_zip},
};
use serde_json::json;
use std::{
    fs,
    io::{Cursor, Write},
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
    time::{Duration, Instant, UNIX_EPOCH},
};

struct Temp(PathBuf);
impl Temp {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "lmp-m3-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn no_download(root: Option<&std::path::Path>) -> ToolManager {
    let mut tools = ToolManager::new(root);
    tools.policy.auto_download = false;
    tools
}

#[test]
fn direct_giphy_and_tenor_resolve_without_tools() {
    let chain = ResolverChain::new(no_download(None));
    let media = chain
        .resolve(
            "https://example.org/a.mp3",
            MediaKind::Audio,
            ResolveOptions::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert!(media.video.is_none());
    assert_eq!(media.audio.unwrap().url, "https://example.org/a.mp3");
    let media = chain
        .resolve(
            "https://giphy.com/gifs/funny-cat-AbCdEf123",
            MediaKind::Image,
            ResolveOptions::default(),
            &Cancellation::default(),
        )
        .unwrap();
    assert_eq!(
        media.video.unwrap().url,
        "https://media.giphy.com/media/AbCdEf123/giphy.gif"
    );
    assert!(GiphyResolver::resolve("https://giphy.com.evil.test/gifs/AbCdEf123").is_err());
    for html in [
        r#"<img src="https://media1.tenor.com/m/abcdEF_123/cat.gif">"#,
        r#"<meta itemprop='contentUrl' content='https://media.tenor.com/m/abcdEF_123/cat.mp4'>"#,
        r#"<meta content='https://media.tenor.com/m/abcdEF_123/cat.mp4' itemprop='contentUrl'>"#,
        r#"{"url":"https:\/\/media.tenor.com\/m\/abcdEF_123\/cat.gif"}"#,
    ] {
        assert_eq!(
            TenorResolver::gif_of(html).unwrap(),
            "https://c.tenor.com/abcdEF_123/tenor.gif"
        );
    }
    assert_eq!(
        TenorResolver::gif_of(
            r#"<meta content='https://example.org/cat.gif?a=1&amp;b=2' property='og:image'>"#
        )
        .unwrap(),
        "https://example.org/cat.gif?a=1&b=2"
    );
    assert!(
        TenorResolver::gif_of(r#"<meta property='og:image' content='file:///cat.gif'>"#).is_none()
    );
    assert!(
        TenorResolver::gif_of(
            r#"<img src='https://media.tenor.com.evil.test/m/abcdEF_123/cat.gif'>"#
        )
        .is_none()
    );
    assert!(
        chain
            .resolve(
                "https://youtube.com/playlist?list=a",
                MediaKind::Video,
                ResolveOptions::default(),
                &Cancellation::default()
            )
            .is_err()
    );
}
#[test]
fn ytdlp_parses_split_muxed_audio_metadata_and_headers() {
    let now = UNIX_EPOCH + Duration::from_secs(1000);
    let split = json!({"title":"Hello 🌍", "duration":12.5, "is_live":true,"thumbnail":"https://img.test/a.jpg",
        "http_headers":{"User-Agent":"test"}, "requested_formats":[
            {"url":"https://cdn.test/video?expire=3000","vcodec":"av1","acodec":"none","http_headers":{"Referer":"https://youtube.com/"}},
            {"url":"https://cdn.test/expire/2000/audio","vcodec":"none","acodec":"opus"}]});
    let media = YtDlpResolver::parse(
        &serde_json::to_vec(&split).unwrap(),
        "https://youtube.com/watch?v=abc",
        MediaKind::Video,
        now,
    )
    .unwrap();
    assert_eq!(media.expires_at, UNIX_EPOCH + Duration::from_secs(2000));
    assert_eq!(media.title.as_deref(), Some("Hello 🌍"));
    assert_eq!(media.duration, Some(12.5));
    assert!(media.is_live);
    assert_eq!(media.video.unwrap().headers.len(), 2);
    assert_eq!(media.audio.unwrap().headers["User-Agent"], "test");
    for (codec, video) in [("h264", true), ("none", false)] {
        let bytes =
            serde_json::to_vec(&json!({"url":"https://cdn.test/media","vcodec":codec})).unwrap();
        let media =
            YtDlpResolver::parse(&bytes, "https://site.test", MediaKind::Video, now).unwrap();
        assert_eq!(media.video.is_some(), video);
        assert_eq!(media.audio.is_some(), !video);
        assert_eq!(media.expires_at, now + Duration::from_secs(3600));
    }
    for value in [
        json!({"url":"file:///etc/passwd"}),
        json!({"url":"https://a.test","http_headers":{"X":"ok\r\nInjected: bad"}}),
        json!({"entries":[]}),
        json!({}),
        json!({"requested_formats":[{"url":"https://a.test","vcodec":"none","acodec":"none"}]}),
    ] {
        assert!(
            YtDlpResolver::parse(
                &serde_json::to_vec(&value).unwrap(),
                "https://a.test",
                MediaKind::Video,
                now
            )
            .is_err()
        );
    }
}
#[test]
fn resolve_cache_expiry_quality_lru_and_403_invalidation() {
    let now = UNIX_EPOCH + Duration::from_secs(1000);
    let cache = ResolveCache::new(2);
    let key = |url: &str, h| ResolveKey {
        url: url.into(),
        options: ResolveOptions {
            max_height: h,
            audio_only: false,
        },
    };
    let mut media =
        DirectResolver::resolve("https://cdn.test/video?expire=2000", MediaKind::Video).unwrap();
    media.expires_at = now + Duration::from_secs(20);
    cache.insert(key("a", 720), media.clone(), now);
    cache.insert(key("a", 1080), media.clone(), now);
    assert!(cache.get(&key("a", 720), now).is_some());
    cache.insert(key("b", 720), media.clone(), now);
    assert!(cache.get(&key("a", 1080), now).is_none());
    cache.on_http_status("a", 404);
    assert!(cache.get(&key("a", 720), now).is_some());
    cache.on_http_status("a", 403);
    assert!(cache.get(&key("a", 720), now).is_none());
    assert!(
        cache
            .get(&key("b", 720), now + Duration::from_secs(20))
            .is_none()
    );
    assert_eq!(
        url_expiry("https://cdn.test/expire/123/x?expire=456"),
        Some(UNIX_EPOCH + Duration::from_secs(123))
    );
    assert!(url_expiry("https://cdn.test/?expire=oops").is_none());
    assert!(url_expiry("https://cdn.test/?expire=18446744073709551615").is_none());
}
#[test]
fn collection_expansion_skips_bad_entries_and_caps_at_500() {
    let collection = SourceRegistry::default()
        .identify("https://youtube.com/playlist?list=abc")
        .unwrap();
    let mut entries = vec![
        json!(null),
        json!({"url":"file:///secret"}),
        json!({"id":"abcdef12345", "ie_key":"Youtube"}),
        json!({"url":"https://soundcloud.com/user/song"}),
    ];
    entries.extend((0..550).map(|i| json!({"url":format!("https://example.org/media/{i}")})));
    let result = CollectionExpander::parse(
        &serde_json::to_vec(&json!({"title":"Playlist", "entries":entries})).unwrap(),
        &collection,
    )
    .unwrap();
    assert_eq!(result.title.as_deref(), Some("Playlist"));
    assert_eq!(result.entries.len(), 500);
    assert_eq!(
        result.entries[0].url,
        "https://www.youtube.com/watch?v=abcdef12345"
    );
    assert_eq!(result.entries[1].kind, MediaKind::Audio);
    assert_eq!(result.entries[2].kind, MediaKind::Video);
    assert!(CollectionExpander::parse(b"{\"entries\":[]}", &collection).is_err());
}
fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}
#[test]
fn tool_download_hosts_magic_and_archives_are_validated() {
    for url in [
        "https://github.com/yt-dlp/yt-dlp/a",
        "https://release-assets.githubusercontent.com/a",
        "https://evermeet.cx/a",
    ] {
        assert!(allowed_download_url(url));
    }
    for url in [
        "http://github.com/a",
        "https://github.com.evil.test/a",
        "https://user@github.com/a",
        "https://github.com:444/a",
        "file:///a",
    ] {
        assert!(!allowed_download_url(url));
    }
    assert!(executable_magic(b"\x7fELF", "linux"));
    assert!(executable_magic(b"MZxx", "windows"));
    assert!(executable_magic(b"\xcf\xfa\xed\xfe", "macos"));
    assert!(!executable_magic(b"<html>", "linux"));
    assert!(!executable_magic(b"MZxx", "linux"));
    assert_eq!(
        extract_zip(
            Cursor::new(zip(&[("folder/bin/ffmpeg", b"\x7fELFtest")])),
            "ffmpeg"
        )
        .unwrap(),
        b"\x7fELFtest"
    );
    for name in [
        "../ffmpeg",
        "/tmp/ffmpeg",
        "C:\\ffmpeg",
        "folder/../../ffmpeg",
    ] {
        assert!(extract_zip(Cursor::new(zip(&[(name, b"bad")])), "ffmpeg").is_err());
    }
    assert!(
        extract_zip(
            Cursor::new(zip(&[("a/ffmpeg", b"a"), ("b/ffmpeg", b"b")])),
            "ffmpeg"
        )
        .is_err()
    );
    assert!(extract_zip(Cursor::new(zip(&[("README", b"text")])), "ffmpeg").is_err());
}
fn pam(w: u32, h: u32, pixels: &[u8]) -> Vec<u8> {
    let mut bytes =
        format!("P7\nWIDTH {w}\nHEIGHT {h}\nDEPTH 4\nMAXVAL 255\nTUPLTYPE RGB_ALPHA\nENDHDR\n")
            .into_bytes();
    bytes.extend(pixels);
    bytes
}
#[test]
fn pam_rejects_truncation_invalid_headers_and_limits() {
    let frame = pam(1, 1, &[1, 2, 3, 4]);
    let image = FfmpegImageDecoder::read_pam(Cursor::new(frame.clone()), None).unwrap();
    assert_eq!(image.frames[0].rgba, [1, 2, 3, 4]);
    let mut animated = frame.clone();
    animated.extend(&frame);
    let image =
        FfmpegImageDecoder::read_pam(Cursor::new(animated.clone()), Some(&[40, 120])).unwrap();
    assert_eq!(image.frames[1].delay_ms, 120);
    assert!(FfmpegImageDecoder::read_pam(Cursor::new(animated), None).is_err());
    for bytes in [
        frame[..frame.len() - 1].to_vec(),
        pam(9000, 1, &[]),
        pam(0, 1, &[]),
        b"P7\nWIDTH nope\nENDHDR\n".to_vec(),
        b"P7\n"
            .iter()
            .copied()
            .chain(std::iter::repeat_n(b'a', 5000))
            .collect(),
    ] {
        assert!(FfmpegImageDecoder::read_pam(Cursor::new(bytes), None).is_err());
    }
    assert!(FfmpegImageDecoder::read_pam(Cursor::new(frame), Some(&[1, 2])).is_err());
    let bytes = (0..257)
        .flat_map(|_| pam(1, 1, &[0; 4]))
        .collect::<Vec<_>>();
    assert!(FfmpegImageDecoder::read_pam(Cursor::new(bytes), Some(&[20; 257])).is_err());
}

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    fn script(path: &std::path::Path, body: &str) {
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[test]
    fn process_drains_stderr_bounds_output_times_out_and_cancels() {
        let cancel = Cancellation::default();
        let out = ProcessRunner::run(Command::new("sh").args(["-c", "i=0; while [ $i -lt 10000 ]; do echo diagnostic >&2; i=$((i+1)); done; printf done"]),Duration::from_secs(5),32,&cancel).unwrap();
        assert_eq!(out, b"done");
        let e = ProcessRunner::run(
            Command::new("sh").args(["-c", "printf output-too-large; sleep 10"]),
            Duration::from_secs(5),
            4,
            &cancel,
        )
        .unwrap_err();
        assert!(e.to_string().contains("exceeds limit"));
        let start = Instant::now();
        let e = ProcessRunner::run(
            Command::new("sh").args(["-c", "sleep 10"]),
            Duration::from_millis(50),
            32,
            &cancel,
        )
        .unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(2));
        let token = cancel.clone();
        let task = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            token.cancel();
        });
        assert!(
            ProcessRunner::run(
                Command::new("sh").args(["-c", "sleep 10"]),
                Duration::from_secs(5),
                32,
                &cancel
            )
            .unwrap_err()
            .to_string()
            .contains("cancelled")
        );
        task.join().unwrap();
        let e = ProcessRunner::run(
            Command::new("sh").args(["-c", "printf failure >&2; exit 7"]),
            Duration::from_secs(2),
            32,
            &Cancellation::default(),
        )
        .unwrap_err();
        assert!(e.to_string().contains("failure"));
    }
    #[test]
    fn process_drop_reaps_child_and_kills_descendants() {
        let temp = Temp::new();
        let pidfile = temp.0.join("pid");
        let mut command = Command::new("sh");
        command
            .args(["-c", "sleep 30 & echo $! > \"$1\"; wait", "test"])
            .arg(&pidfile);
        let process = ManagedProcess::spawn(&mut command).unwrap();
        let pid = process.id();
        let start = Instant::now();
        while !pidfile.exists() || fs::read_to_string(&pidfile).unwrap().trim().is_empty() {
            assert!(start.elapsed() < Duration::from_secs(2));
            std::thread::sleep(Duration::from_millis(5));
        }
        let child = fs::read_to_string(&pidfile).unwrap();
        drop(process);
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
        // SIGKILL delivery to the grandchild is asynchronous with reaping the parent.
        // Wait for death, accepting a zombie pending init's reaper, but never a survivor.
        let deadline = Instant::now() + Duration::from_secs(2);
        while let Ok(stat) = fs::read_to_string(format!("/proc/{}/stat", child.trim())) {
            if stat.split(") ").nth(1).unwrap().starts_with('Z') {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "grandchild survived process-group cleanup"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    #[test]
    fn closing_an_image_cancels_its_running_decoder() {
        use lias_media_player::{
            core::{cache::DiskCache, images::Images},
            platform::horizon_impl::ModAtlasPacker,
            ui::WindowManager,
        };
        let temp = Temp::new();
        let bin = temp.0.join("bin");
        fs::create_dir(&bin).unwrap();
        let pidfile = temp.0.join("decoder-pid");
        script(
            &bin.join("ffmpeg"),
            &format!(
                "if [ \"$1\" = '-version' ]; then echo 'ffmpeg version test'; else echo $$ > '{}'; sleep 30; fi",
                pidfile.display()
            ),
        );
        let mut tools = no_download(Some(&temp.0));
        tools.policy.prefer_system = false;
        let url = "https://example.org/image.apng";
        DiskCache::open(&temp.0.join("cache"), 1024 * 1024)
            .unwrap()
            .put(url, include_bytes!("fixtures/animated.apng"))
            .unwrap();
        let mut images = Images::with_tools(Some(&temp.0), 1024 * 1024, 1024 * 1024, tools);
        let mut windows = WindowManager::new();
        let mut atlas = ModAtlasPacker::new();
        let id = images
            .request(url, true, &mut windows, &mut atlas, None)
            .unwrap();
        let start = Instant::now();
        let pid = loop {
            if let Ok(value) = fs::read_to_string(&pidfile)
                && let Ok(pid) = value.trim().parse::<u32>()
            {
                break pid;
            }
            assert!(start.elapsed() < Duration::from_secs(3));
            std::thread::sleep(Duration::from_millis(5));
        };
        windows.close(id);
        images.tick(Duration::ZERO, &windows, &mut atlas, None);
        let start = Instant::now();
        while std::path::Path::new(&format!("/proc/{pid}")).exists() {
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "decoder survived window close"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(images.view(id).is_none());
    }
    #[test]
    fn tool_lookup_and_resolver_execute_safe_arguments_with_fake_ytdlp() {
        let temp = Temp::new();
        let bin = temp.0.join("bin");
        fs::create_dir(&bin).unwrap();
        let output = temp.0.join("output.json");
        let args = temp.0.join("args");
        script(
            &bin.join("yt-dlp"),
            &format!(
                "if [ \"$1\" = '--version' ]; then echo 2026.11.01; else printf '%s\\n' \"$@\" > '{}'; cat '{}'; fi",
                args.display(),
                output.display()
            ),
        );
        let mut tools = no_download(Some(&temp.0));
        tools.policy.prefer_system = false;
        assert_eq!(
            tools
                .require(Tool::YtDlp, &Cancellation::default())
                .unwrap(),
            bin.join("yt-dlp")
        );
        assert!(
            tools
                .require(Tool::Ffmpeg, &Cancellation::default())
                .is_err()
        );
        fs::write(
            &output,
            br#"{"url":"https://cdn.test/video","title":"test","vcodec":"h264"}"#,
        )
        .unwrap();
        let chain = ResolverChain::new(tools.clone());
        for url in [
            "https://youtube.com/watch?v=abc&x=$(oops)",
            "https://twitch.tv/channel",
            "https://reddit.com/r/a/comments/b/c",
            "https://soundcloud.com/person/song",
        ] {
            let media = chain
                .resolve(
                    url,
                    MediaKind::Video,
                    ResolveOptions::default(),
                    &Cancellation::default(),
                )
                .unwrap();
            assert_eq!(media.title.as_deref(), Some("test"));
            let text = fs::read_to_string(&args).unwrap();
            assert!(text.starts_with("--ignore-config\n"));
            assert!(text.ends_with(&format!("--\n{url}\n")));
            if url.contains("soundcloud") {
                assert!(text.contains("ba/b\n"));
            } else {
                assert!(text.contains("height<=720"));
            }
        }
        let url = "https://youtube.com/watch?v=abc&x=$(oops)";
        fs::write(&output, b"invalid").unwrap();
        assert!(
            chain
                .resolve(
                    url,
                    MediaKind::Video,
                    ResolveOptions::default(),
                    &Cancellation::default()
                )
                .is_ok()
        );
        chain.on_http_status(url, 403);
        assert!(
            chain
                .resolve(
                    url,
                    MediaKind::Video,
                    ResolveOptions::default(),
                    &Cancellation::default()
                )
                .is_err()
        );
        fs::write(
            &output,
            br#"{"entries":[{"id":"abcdef12345","ie_key":"Youtube"}]}"#,
        )
        .unwrap();
        let collection = SourceRegistry::default()
            .identify("https://youtube.com/playlist?list=abc")
            .unwrap();
        assert_eq!(
            CollectionExpander::expand(&tools, &collection, &Cancellation::default())
                .unwrap()
                .entries
                .len(),
            1
        );
        assert!(
            fs::read_to_string(&args)
                .unwrap()
                .contains("--playlist-end\n500\n")
        );
        let mut legacy = no_download(None);
        legacy.policy.prefer_system = false;
        legacy.legacy_bin = Some(bin);
        assert!(
            legacy
                .require(Tool::YtDlp, &Cancellation::default())
                .is_ok()
        );
    }
}
#[test]
fn real_ffmpeg_decodes_apng_and_avif_fixtures() {
    let tools = no_download(None);
    if tools
        .locate(Tool::Ffmpeg, &Cancellation::default())
        .is_none()
    {
        eprintln!("SKIP: FFmpeg unavailable; install on PATH to run image integration");
        return;
    }
    let apng = include_bytes!("fixtures/animated.apng");
    assert!(FfmpegImageDecoder::supports(apng));
    let image = ImageLoader::decode_with_tools(apng, &tools, &Cancellation::default()).unwrap();
    assert_eq!((image.width, image.height), (4, 3));
    assert_eq!(
        image.frames.iter().map(|f| f.delay_ms).collect::<Vec<_>>(),
        [40, 120, 260]
    );
    assert_eq!(
        image
            .frames
            .iter()
            .flat_map(|f| f.rgba.clone())
            .collect::<Vec<_>>(),
        include_bytes!("fixtures/animated.apng.raw")
    );
    let image = ImageLoader::decode_with_tools(
        include_bytes!("fixtures/still.avif"),
        &tools,
        &Cancellation::default(),
    )
    .unwrap();
    assert_eq!((image.width, image.height, image.frames.len()), (4, 3, 1));
    for pixel in image.frames[0].rgba.as_chunks::<4>().0.iter() {
        assert!(pixel[0] > 245 && pixel[1] < 10 && pixel[2] < 10 && pixel[3] == 255);
    }
}

#[test]
fn apng_loads_through_runtime_and_restores_from_disk_cache() {
    use std::{io::Read, net::TcpListener};
    let tools = no_download(None);
    if tools
        .locate(Tool::Ffmpeg, &Cancellation::default())
        .is_none()
    {
        return;
    }
    let temp = Temp::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/image.apng", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let (mut socket, _) = listener.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let _ = socket.read(&mut [0; 4096]);
        let body = include_bytes!("fixtures/animated.apng");
        write!(
            socket,
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        socket.write_all(body).unwrap();
    });
    for load in 0..2 {
        // A fresh runtime ensures the second load goes through the encoded disk cache,
        // including FFmpeg decode, rather than the in-memory image cache.
        let mut runtime = lias_media_player::Runtime::new();
        runtime.configure(&temp.0, &temp.0.join("java"));
        if load == 0 {
            runtime.execute_show(
                lias_media_player::command::parse(&format!("show image {url}"))
                    .unwrap()
                    .unwrap(),
            );
        }
        assert_eq!(runtime.ui.windows.windows().len(), 1);
        let id = runtime.ui.windows.windows()[0].id;
        let view = runtime.images.as_ref().unwrap().view(id).unwrap();
        let started = Instant::now();
        loop {
            runtime.on_frame(Duration::from_millis(5));
            if view.lock().image.is_some() {
                break;
            }
            assert!(
                started.elapsed() < Duration::from_secs(5),
                "{}",
                view.lock().status
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(view.lock().image.as_ref().unwrap().frames.len(), 3);
        assert!(
            runtime
                .ui
                .atlas
                .region(view.lock().texture.unwrap())
                .is_some()
        );
        runtime.on_world_unload();
        // The persistence layer is debounced; flush before reconstructing the runtime.
        runtime.stores.as_mut().unwrap().flush().unwrap();
    }
    server.join().unwrap();
}

/// Opt-in network smoke test: never downloads an executable during the default suite.
#[test]
#[ignore = "downloads and executes the official yt-dlp release over HTTPS"]
fn live_ytdlp_download() {
    let temp = Temp::new();
    let mut tools = ToolManager::new(Some(&temp.0));
    tools.policy.prefer_system = false;
    let path = tools
        .require(Tool::YtDlp, &Cancellation::default())
        .unwrap();
    assert_eq!(path, temp.0.join("bin").join(Tool::YtDlp.name()));
    assert_eq!(fs::read_dir(temp.0.join("bin")).unwrap().count(), 1);
    let version = ProcessRunner::run(
        Command::new(path).arg("--version"),
        Duration::from_secs(5),
        1024,
        &Cancellation::default(),
    )
    .unwrap();
    eprintln!(
        "Downloaded yt-dlp {}",
        String::from_utf8_lossy(&version).trim()
    );
}

#[test]
fn youtube_collection_entries_keep_titles_and_resolve_individually() {
    let mut collection = SourceRegistry::default()
        .identify("https://music.youtube.com/watch?v=one&list=example")
        .unwrap();
    collection.entry_kind = MediaKind::Audio;
    let result = CollectionExpander::parse(
        br#"{"title":"Mix","entries":[null,{"title":"[Deleted video]","url":"https://youtu.be/deleted"},{"title":"Private","availability":"private","url":"https://youtu.be/private"},{"title":"A real song","url":"https://www.youtube.com/watch?v=one&list=example&index=1"}]}"#,
        &collection,
    ).unwrap();
    assert_eq!(result.entries.len(), 1);
    let song = &result.entries[0];
    assert_eq!(song.label, "A real song");
    assert_eq!(song.url, "https://www.youtube.com/watch?v=one");
    assert_eq!(song.kind, MediaKind::Audio);
    assert_eq!(
        SourceRegistry::default().identify(&song.url).unwrap().kind,
        MediaKind::Video
    );
}
