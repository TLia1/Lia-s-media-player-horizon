// Example addon compiled into horizon-client alongside Lia's Media Player.
use lias_media_player::api::*;

struct Radio;
impl LinkSource for Radio {
    fn id(&self) -> &str {
        "example_radio:station"
    }
    fn priority(&self) -> i32 {
        20
    }
    fn matches(&self, url: &Url) -> Option<(MediaKind, MediaKind, &str)> {
        (url.host_str() == Some("radio.example.org") && url.path() == "/listen").then_some((
            MediaKind::Audio,
            MediaKind::Audio,
            "radio",
        ))
    }
}
impl Resolver for Radio {
    fn id(&self) -> &str {
        "example_radio:station"
    }
    fn resolve(
        &self,
        request: ResolveRequest<'_>,
        cancel: &Cancellation,
    ) -> Option<std::io::Result<ResolvedMedia>> {
        let url = Url::parse(request.url).ok()?;
        self.matches(&url)?;
        if cancel.is_cancelled() {
            return Some(Err(std::io::Error::other("Cancelled")));
        }
        Some(Ok(ResolvedMedia {
            source_url: request.url.into(),
            webpage_url: Some(request.url.into()),
            kind: MediaKind::Audio,
            video: None,
            audio: Some(StreamUrl {
                url: "https://radio.example.org/live.mp3".into(),
                headers: Default::default(),
            }),
            title: Some("Example Radio".into()),
            thumbnail: None,
            duration: None,
            is_live: true,
            expires_at: std::time::SystemTime::now() + std::time::Duration::from_secs(60),
        }))
    }
}
struct ExampleAddon;
impl MediaPlayerAddon for ExampleAddon {
    fn id(&self) -> &str {
        "example_radio"
    }
    fn register(&self, context: &mut AddonContext) -> Result<(), RegisterError> {
        context.source(Radio);
        context.resolver(Radio);
        context.theme(
            ThemeSpec::new("example_radio:violet", "Radio Violet")
                .with(ThemeRole::Fill, 0xffbb88ff),
        );
        Ok(())
    }
}
pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Err(error) = register_addon(Box::new(ExampleAddon)) {
            eprintln!("Example Radio: {error}");
        }
    });
}
