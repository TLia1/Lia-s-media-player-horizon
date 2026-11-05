# Rust addon API 4

This reference describes the API implemented in
[`src/api`](../mixins/liasmediaplayer/lias_media_player/src/api/), for mod version
**4.0.0**. Import it with:

```rust
use lias_media_player::api::*;
```

`API_VERSION` is `4`. This is a Rust source API compiled into the same Horizon
executable as the player. It is not a binary ABI, dynamically loaded plugin
format, HTTP API, or compatibility layer for Java addons. The crate is currently
`publish = false`; there is no crates.io installation workflow for this release.

- [Loading and registration](#loading-and-registration)
- [Link sources](#link-sources)
- [Resolvers](#resolvers)
- [Themes and window actions](#themes-and-window-actions)
- [Opening and controlling media](#opening-and-controlling-media)
- [Snapshots](#snapshots)
- [Events](#events)
- [Threads, lifecycle, and errors](#threads-lifecycle-and-errors)
- [Coming from the Java mod](#coming-from-the-java-mod)

## Loading and registration

On the pinned hooks API 359 / horizon-mixins `e8cd0aa` target, the player is
staged as a crate dependency of `horizon-client`. Addons inject a support module
into that same client crate and import its existing `lias_media_player`
dependency. Both mods must be included when Horizon is compiled.

The working layout is in [`examples/addon`](../examples/addon/):

| File | Role |
| --- | --- |
| `horizon-mixin.toml` | Declares the addon module in `crates/horizon-client/src/game.rs` and a `mods` catalog hook |
| `init.rs` | Calls the injected module's installer from the hook |
| `addon.rs` | Implements extensions and registers once using `std::sync::Once` |

Follow the [combined-build instructions](BUILDING.md#build-the-example-addon-with-the-player).
The pinned weaver does not accept `[api]` or mod `[dependencies]` tables. It also
cannot express an optional dependency on this native mod. Building the addon
alone fails; duplicating the player crate is not a substitute for sharing its
registry and runtime.

### Minimal addon

```rust
use lias_media_player::api::*;

struct MyAddon;

impl MediaPlayerAddon for MyAddon {
    fn id(&self) -> &str {
        "my_addon"
    }

    fn register(&self, context: &mut AddonContext) -> Result<(), RegisterError> {
        context.theme(
            ThemeSpec::new("my_addon:violet", "Violet")
                .with(ThemeRole::Fill, 0xffbb88ff),
        );
        context.window_action(WindowAction {
            id: "my_addon:restart".into(),
            label: "R".into(),
        });
        Ok(())
    }
}

pub fn install() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        if let Err(error) = register_addon(Box::new(MyAddon)) {
            eprintln!("My addon could not register: {error}");
        }
    });
}
```

The installer must actually be called by a host hook; defining it does not load
the addon. Registration installs extensions but does not enable the player or
open media. `Once` prevents repeated registration, but also prevents an automatic
retry if the first attempt fails.

### Registration contract

```rust
pub trait MediaPlayerAddon: Send + Sync {
    fn id(&self) -> &str;
    fn api_version(&self) -> u32 { API_VERSION }
    fn register(&self, context: &mut AddonContext) -> Result<(), RegisterError>;
}
```

`register_addon(Box<dyn MediaPlayerAddon>) -> Result<(), RegisterError>` is also
re-exported at the crate root. The API version must equal `API_VERSION` exactly.
`RegisterError(pub String)` implements `Display` and `std::error::Error`.

The addon ID is a **namespace**, such as `my_addon`, with 1–64 ASCII lowercase
letters, digits, or underscores. The `liasmediaplayer` namespace is reserved.
Each extension ID must be `my_addon:name`, with the same character and length
rules for `name`. Extension IDs are unique **within each extension category**;
a source and resolver may share an ID, as the radio example does.

| `AddonContext` method | Accepted value |
| --- | --- |
| `source(source)` | `impl LinkSource + 'static` |
| `resolver(resolver)` | `impl Resolver + 'static` |
| `theme(theme)` | `ThemeSpec` |
| `window_action(action)` | `WindowAction` |

These methods stage values and return `()`. Validation and installation occur
when `register_addon` completes. A returned error, a caught panic inside
`register`, invalid IDs, duplicate registration, version mismatch, or exceeding
the action limit commits none of the staged extensions. External side effects
performed by addon code cannot be rolled back. Do not panic in `id()`,
`api_version()`, source metadata methods, or runtime callbacks: the panic guard
does not cover every call into addon code.

Late registration is supported, but register early to restore a saved addon
theme during runtime configuration. There is no unregister or hot-unload API.

## Link sources

A source classifies a URL and supplies its chat label. It does not fetch media.

```rust
pub trait LinkSource: Send + Sync {
    fn id(&self) -> &str;
    fn priority(&self) -> i32 { 0 }
    fn matches(&self, url: &Url) -> Option<(MediaKind, MediaKind, &str)>;
}
```

`Url` is re-exported from the `url` crate. The match tuple is
**`(kind, entry_kind, label)`**. Use the same kind twice for individual media;
use `Collection` as `kind` and the member kind as `entry_kind` for collections.
Return `None` to decline.

```rust
use lias_media_player::api::*;

struct StationSource;
impl LinkSource for StationSource {
    fn id(&self) -> &str { "my_addon:station" }
    fn priority(&self) -> i32 { 20 }

    fn matches(&self, url: &Url) -> Option<(MediaKind, MediaKind, &str)> {
        (url.host_str() == Some("radio.example.org") && url.path() == "/listen")
            .then_some((MediaKind::Audio, MediaKind::Audio, "radio"))
    }
}
```

Register this with `context.source(StationSource)`. Use parsed host/path checks
instead of raw URL prefix matching. Accepted web URLs have an HTTP(S) scheme,
a host, no embedded username/password, and no control characters.

Higher priority runs first. Addons precede built-ins at equal priority; earlier
addon registration breaks equal-priority ties. The first match wins. Existing
source registries see newly registered sources. Matching can run on the client
thread: keep it fast, nonblocking, and free of side effects or runtime calls.

The resulting `MediaRef` has public fields:

| Field | Type | Meaning |
| --- | --- | --- |
| `url` | `String` | Original input URL |
| `kind` | `MediaKind` | `Image`, `Video`, `Audio`, or `Collection` |
| `source_id` | `String` | ID of the matching source |
| `label` | `String` | Display label |
| `entry_kind` | `MediaKind` | Member kind for collections |

Recognizing a new collection does not install a collection expander. Collection
expansion remains a built-in service; there is no addon expander registry in API 4.

## Resolvers

Resolvers turn page URLs into playable streams on background workers:

```rust
pub trait Resolver: Send + Sync {
    fn id(&self) -> &str;
    fn resolve(
        &self,
        request: ResolveRequest<'_>,
        cancel: &Cancellation,
    ) -> Option<std::io::Result<ResolvedMedia>>;
}
```

Register with `context.resolver(MyResolver)`. See the complete
[`Radio` resolver](../examples/addon/addon.rs) for an implementation.

| Return value | Effect |
| --- | --- |
| `None` | Decline; try the next resolver |
| `Some(Ok(media))` | Handle the request; validate and use the result |
| `Some(Err(error))` | Fail this resolution; do not fall back to another resolver |

Addon resolvers run in registration order before built-in resolution and its
cache lookup. A resolver can be called concurrently for multiple players. Honor
`cancel.is_cancelled()`, use finite I/O deadlines, and release resources promptly.
`Cancellation` is cloneable; clones share a flag, and `cancel()` sets it.
The host checks cancellation after a handled result but cannot interrupt an
addon that blocks forever.

### Requests and results

`ResolveRequest<'a>` contains `url: &'a str`, `kind: MediaKind`, and
`options: ResolveOptions`. Source classification can determine the requested
kind; audio intent is preserved for non-collection playback. Match the request
actually received, including its kind.

`ResolveOptions` has `max_height: u32` and `audio_only: bool`; its default is
720 and false. Respect those preferences when choosing streams.

`StreamUrl` contains `url: String` and `headers: BTreeMap<String, String>`.
`ResolvedMedia` contains:

| Field | Type | Meaning |
| --- | --- | --- |
| `source_url` | `String` | Set to the original request URL by the host |
| `webpage_url` | `Option<String>` | Canonical page URL |
| `kind` | `MediaKind` | Must equal the request kind |
| `video` | `Option<StreamUrl>` | Video/muxed stream; also used for image results |
| `audio` | `Option<StreamUrl>` | Audio stream, including separate audio for video |
| `title` | `Option<String>` | Display title |
| `thumbnail` | `Option<String>` | Preview URL |
| `duration` | `Option<f64>` | Duration in seconds, if known |
| `is_live` | `bool` | Whether the stream is live |
| `expires_at` | `std::time::SystemTime` | Stream expiry metadata |

At least one stream is required. Stream URLs must pass the HTTP(S) URL check.
Header names must be nonempty and contain no colon/CR/LF; values must contain no
CR/LF. A duration must be finite and nonnegative. Invalid handled results produce
an error rather than falling through. Supply appropriate streams for the kind,
not merely any nonempty stream.

Addon resolvers own caching and expiry decisions: their results are not inserted
into the built-in resolver cache. Existing encoded-image/preview caches may
satisfy already-loaded URLs without another resolution. Image downloads do not
currently forward custom stream headers. Collection URLs are rejected by this
resolution path before addon resolvers run; expansion is separate.

## Themes and window actions

`ThemeSpec` has `id: String`, `name: String`, and
`colors: BTreeMap<ThemeRole, u32>`. Construct it with
`ThemeSpec::new(id, name)`, chain `.with(role, color)`, and read a resolved color
with `.color(role)`. Colors use **`0xAARRGGBB`**. Unspecified roles inherit the dark
palette. `themes() -> Vec<ThemeSpec>` returns built-ins followed by addon themes.

The roles are:

```text
WindowBg, TitleBarBg, ControlBarBg, Placeholder, PanelBg, ListBg,
RowBg, RowHover, RowSelected, FieldBg, ChipBg, ChipHover, BannerBg,
Border, BorderFocused, Track, Fill, Knob, ScrollTrack, ScrollThumb,
Text, TextSubtle, TextDim, Icon, IconHover, IconActive, IconDisabled,
Danger, Muted, Spinner
```

Registered themes appear in Settings and their IDs can be persisted. Register
before runtime configuration to apply a saved addon theme automatically.

`WindowAction { id: String, label: String }` adds a window title-bar button.
The label must be exactly one Unicode character that is not a control character.
There are at most **four addon actions globally**, across all addons.
`window_actions() -> Vec<WindowAction>` returns the registered actions.
Clicking sends `MediaEvent::WindowAction { id, handle }`; no addon function is
called directly from the button. Subscribe and handle that event outside host
callbacks, as shown below.

## Opening and controlling media

```rust
pub fn open(url: &str, kind: MediaKind) -> Result<MediaHandle, String>;
pub fn active_media() -> Result<Vec<MediaSnapshot>, String>;
```

These declarations show signatures; use the functions exported by the crate.
`open` always requests a separate window, subject to the normal UI window limits
and tool policy. Accepted kinds are `Image`, `Video`, and `Audio`. `Collection`
is rejected; supply already-expanded track URLs to queue commands instead.
GIFs and other animated images use `MediaKind::Image`.

The mod must be enabled and in a world. A successful return means the window
request was admitted; resolution, downloads, decoding, and output can still fail
later. Inspect snapshots or events for the outcome.

```rust
use lias_media_player::api::*;

fn play_track(url: &str) -> Result<MediaHandle, String> {
    let handle = open(url, MediaKind::Audio)?;
    handle.command(PlayerCommand::SetVolume(0.5))?;
    handle.window(WindowCommand::Pin(true))?;
    Ok(handle)
}
```

### `MediaHandle`

A handle is `Copy`, `Clone`, comparable, orderable, and hashable. Its numeric ID
is local to this runtime session and is not reused. Do not persist it as a way
to find media after restarting the game.

| Method | Return type | Effect |
| --- | --- | --- |
| `id(self)` | `u64` | Read the session ID |
| `snapshot(self)` | `Result<MediaSnapshot, String>` | Read current media/window state |
| `command(self, PlayerCommand)` | `Result<(), String>` | Submit transport/queue work |
| `window(self, WindowCommand)` | `Result<(), String>` | Change window state |

Window commands are `Show`, `Hide`, `Pin(bool)`, `Theatre(bool)`, and `Close`.
Hide does not close a player; the configured hidden-video policy still applies.
Close removes the window and invalidates the handle. Use `window(Close)` to close
an image; image transport only accepts `Play`, `Pause`, and `Toggle`.

### Audio/video `PlayerCommand`

| Command | Behavior |
| --- | --- |
| `Load(String)` | Clear the queue/history and load an individual media URL in this player |
| `Enqueue(String)` | Append one URL; start it if the player is idle/ended |
| `EnqueueBatch(Vec<String>)` | Append validated URLs atomically, subject to the queue limit |
| `Play`, `Pause`, `Toggle` | Control playback |
| `Seek(f64)` | Absolute time in seconds; finite values are clamped to the available duration; live seeking is ignored |
| `SetSpeed(f64)` | Finite values are clamped to 0.25–4.0 |
| `SetVolume(f32)` | Finite values are clamped to 0.0–1.0 |
| `SetLoop(bool)` | Repeat the current item; enabling it disables repeat-all |
| `SetRepeatAll(bool)` | Repeat the queue; enabling it disables single-item looping |
| `CycleRepeat` | Off → all → one → off |
| `SetShuffle(bool)` | Enable/disable queue shuffle |
| `SetTitles(Vec<(String, String)>)` | Supply `(URL, title)` pairs; at most 500 pairs per command and 256 characters per title |
| `Previous`, `Next` | Move through playback history/queue |
| `JumpQueue(usize)` | Remove and play the queued item at a zero-based index |
| `MoveQueue(usize, usize)` | Move a queued item between zero-based indices |
| `RemoveQueue(usize)` | Remove a queued item |
| `ClearQueue` | Clear queued items and previous-item history |
| `SetTargetHeight(u32)` | Choose 360, 480, 720, or 1080 lines |
| `SetFps(u32)` | Choose 30 or 60 FPS |
| `SetVideo(bool)` | Enable/disable video decoding on a video player |
| `Retry` | Clear the player's resolved/probed state and retry playback |
| `Suspend` | Release the active session and pause; used for lifecycle suspension |
| `Stop` | Stop the session, clear the queue, and reset position/state; keep the window |

Queue capacity is **500**. Commands do not expand collection URLs. Unsupported
numeric values can be ignored by the actor rather than rejected at submission.
For audio/video, `Ok(())` acknowledges mailbox admission, not successful execution.
The command mailbox is bounded (64 messages); a full or closed mailbox returns an
error. Observe subsequent snapshots for actual state and actor-side errors.

## Snapshots

`MediaSnapshot` is a cloned view, not a live mutable object:

| Field | Type |
| --- | --- |
| `handle` | `MediaHandle` |
| `kind` | `MediaKind` |
| `source_url`, `title` | `String` |
| `state` | `PlaybackState` |
| `hidden`, `pinned` | `bool` |
| `error` | `Option<String>` |
| `playback` | `Option<Arc<PlayerSnapshot>>` |

Images have `playback: None`. Audio/video snapshots include:

| `PlayerSnapshot` fields | Types / units |
| --- | --- |
| `state`, `source_url`, `title`, `error` | `PlaybackState`, `String`, `String`, `Option<String>` |
| `position_secs`, `duration_secs` | `f64` seconds, `Option<f64>` seconds |
| `is_live` | `bool` |
| `video_size` | `Option<(u32, u32)>`, width/height |
| `speed`, `volume` | `f64` multiplier, `f32` gain |
| `looping`, `shuffle`, `repeat_all` | `bool` |
| `target_height`, `fps` | `u32` |
| `sound_only` | `bool` |
| `queue`, `queue_titles` | `Vec<String>` URLs and corresponding titles |
| `has_previous` | `bool` |
| `generation` | `u64`, actor generation counter |

`PlaybackState` variants are `Idle`, `Resolving`, `Buffering`, `Playing`, `Paused`,
`Ended`, and `Failed`. Do not assume a fixed sequence or that every transition
will be observed. Call `snapshot()` again to refresh position and controls.

## Events

```rust
pub fn subscribe(capacity: usize) -> Result<EventSubscription, String>;
```

Capacity is **clamped** to 1–1024; there are at most **64 subscribers**.
`EventSubscription` dereferences to `std::sync::mpsc::Receiver<MediaEvent>`, so
`try_recv()`, `try_iter()`, `recv_timeout()`, and ordinary receiver methods are
available. Drop the subscription to unsubscribe and release its admission slot.

| Event | Meaning |
| --- | --- |
| `Opened(MediaSnapshot)` | A media window appeared in the observer's frame sample |
| `Changed(MediaSnapshot)` | State, source URL, title, hidden/pinned flag, or error changed |
| `Closed(MediaHandle)` | A previously observed window is no longer present |
| `WindowAction { id: String, handle: MediaHandle }` | An addon window action was clicked |

Lifecycle/state events are sampled once per host frame. Intermediate transitions
can be missed, and position-only changes do not emit events. Neither do changes
only to volume, speed, queue, repeat, or theatre mode. Poll snapshots for those.
A subscription is not an initial-state replay: call `active_media()` after
subscribing to establish the current state.

Delivery never waits for consumers. A full receiver is **disconnected**, rather
than silently dropping one event and continuing. Drain any buffered events,
subscribe again, and resynchronize with `active_media()`.

```rust
use lias_media_player::api::*;
use std::sync::mpsc::TryRecvError;

// Call periodically outside callbacks that hold the host runtime lock.
fn poll_actions(events: &EventSubscription) -> Result<(), String> {
    loop {
        match events.try_recv() {
            Ok(MediaEvent::WindowAction { id, handle }) if id == "my_addon:restart" => {
                handle.command(PlayerCommand::Seek(0.0))?;
                handle.command(PlayerCommand::Play)?;
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => return Ok(()),
            Err(TryRecvError::Disconnected) => {
                return Err("Resubscribe and refresh active_media()".into());
            }
        }
    }
}
```

Create the receiver with `let events = subscribe(128)?;` and store it between
polls. This example propagates busy/closed-handle errors; production callers
should decide whether to retry or discard the action. Blocking receiver methods
belong on a dedicated consumer thread, never the render/client thread.

## Threads, lifecycle, and errors

| Operation | Contract |
| --- | --- |
| Registration | Callable from any thread, including a hook already holding the runtime lock; registration callback executes outside registry locks |
| Source matching | Thread-safe, fast, nonblocking; may execute on the client thread |
| Resolution | Worker-side, thread-safe, bounded I/O, cooperative cancellation |
| Runtime access (`open`, snapshots, controls) | Thread-safe; tries the runtime lock and returns an error if it is busy |
| Event consumption | Poll outside host callbacks, or consume on your own worker; no addon event callback is invoked under the runtime lock |

Runtime access reports `String` errors, including unavailable world/mod state,
closed windows, invalid URLs, admission limits, and busy runtime/player state.
Treat error text as diagnostics, not a stable machine-readable error code.
Retry transient busy errors later; do not spin inside a host callback. Resolvers
return `io::Error`, while registration uses `RegisterError`.

On world unload, images close and audio/video playback is suspended. Their
windows disappear from the active view; audio/video window IDs can return on
world entry. A handle unavailable during that transition is not a durable
cross-world resource. Opening media or sending transport commands while disabled
or outside a world returns an error. Registration does not bypass those rules.

Addon implementations are native code in the game process. Runtime source and
resolver panics are not isolated by the registration transaction. Keep callbacks
well-behaved and avoid re-entering runtime controls from source matching.

## Coming from the Java mod

Both implementations use the number 4 for their addon APIs. That does not imply
source, binary, or feature compatibility.

| Java API concept | Rust API in this repository |
| --- | --- |
| Fabric entrypoint / NeoForge registration | Native support module and explicit `register_addon`, compiled with the player |
| `MediaPlayerAddon`, `AddonContext` | Traits/context with transactional registration and explicit namespace IDs |
| Link sources and resolvers | `LinkSource`, `Resolver`; worker resolution returns `Option<io::Result<_>>` |
| Theme registry | `ThemeSpec`, `ThemeRole` |
| Window action callback | One-character button plus `MediaEvent::WindowAction` |
| `MediaPlayer.play`, media handles | `open`, `MediaHandle`, `PlayerCommand`, `WindowCommand` |
| Event listeners | Bounded pull-based subscriptions |
| Queue view | Snapshot fields and queue commands |
| Library/history/settings extension APIs | No supported public addon equivalent |
| Shortcut/keybinding/setting/interceptor registries | Not implemented in the Rust addon API |
| Custom collection expansion / timestamp-link interfaces | No equivalent addon registry/trait |
| Capability negotiation, `onReady`, diagnostics API | Not exposed |
| Surfaces, positional audio, watch-together, custom decoders/tabs | Not exposed |

Although `core`, `ui`, `platform`, and `runtime` are public Rust modules for the
application and tests, they are not the supported addon contract. Prefer the
`api` re-exports and avoid depending on internal runtime structures.

## Testing and generated reference

```sh
cargo test --workspace --locked --test milestone7
cargo doc -p lias_media_player --no-deps --locked
```

The milestone 7 suite compiles the checked-in addon example and tests registration,
resolver integration, validation/cancellation, handles, events, themes, and actions.
It does not replace a native build of your own addon with the player. See
[BUILDING.md](BUILDING.md) for full checks and generated-documentation locations.
