# Lia's Media Player for Horizon

Play videos, audio, images, and animations inside Horizon, with floating players,
chat link previews, playlists, and playback history. This is the Rust rewrite of
Lia's Media Player, distributed as a **client-side Horizon native mod**.

The current version is **4.0.0**, with Rust addon API **4**. Horizon compiles the
mod's Rust sources into its client executable. The `.hnmod` file is a source
bundle; it is not a Fabric/NeoForge JAR or a standalone media-player executable.

- [Build and installation guide](docs/BUILDING.md)
- [Rust API reference](docs/API.md)
- [Addon quick start](docs/addons.md) and [example addon](examples/addon/)

- **Video and audio:** direct HTTP(S) media, website links resolved through
  yt-dlp, FFmpeg decoding, and audio output through CPAL. Built-in link recognition
  covers YouTube, Twitch, Vimeo, Streamable, Reddit, SoundCloud, and Bandcamp.
  Availability depends on the provider and the installed tools.
- **Images and animations:** PNG, JPEG, GIF, BMP, WebP, animated WebP, and
  FFmpeg-assisted formats such as APNG and AVIF. Tenor and Giphy page links are
  resolved to media.
- **Floating players:** move, resize, zoom, hide, pin, and use theatre mode.
  Audio/video players provide seeking, volume, speed, quality, queues, shuffle,
  and repeat. The HUD shows current playback and disappears when no media is open.
- **Chat integration:** recognized links become clickable badges with hover
  previews. Chat also provides playlist and hidden-window controls.
- **Playlists:** create, rename, search, reorder tracks, expand collection links,
  play or queue tracks, shuffle, loop, and import/export M3U through the clipboard.
  Playlist playback uses audio players, including for video links.
- **History and favourites:** replay recent media and keep favourite entries.
- **Settings and persistence:** four built-in themes, tool controls, cache and
  playback settings, saved window state, and import of existing Java-mod data.
- **Rust addons:** register link sources, resolvers, themes, and window buttons;
  open/control media through handles and receive bounded event streams.

## Compatibility

The checked-in manifests declare the following target:

| Component | Target |
| --- | --- |
| Mod ID / version | `liasmediaplayer` / `4.0.0` |
| Game | Java Edition `26.3`, through Horizon |
| Side | Client |
| Horizon revision | `6475f62f3c834b7489e1816bf47f47ea5eb2b879` |
| Native hooks API | `359` |
| horizon-mixins revision | `e8cd0aa38cffef49cb2e088ef716baa107148c02` |
| Mixin schema | `6` |
| Rust addon API | `4` (`lias_media_player::api::API_VERSION`) |

See [compatibility.json](compatibility.json),
[horizon-native.json](horizon-native.json), and the
[mixin manifest](mixins/liasmediaplayer/horizon-mixin.toml).
Some integrations target Horizon internals directly, so a newer Horizon release
needs verification even if it has more hooks. The implementation guide includes
historical smoke-test results on other revisions, not a promise of compatibility
with every Nightly. Its proposed API 409 design is not the current build target.

## Build and install

Install and prepare a Horizon instance first. From a checkout of this repository,
with `horizon-launcher` on `PATH`:

```sh
horizon-launcher native-build "$PWD/mixins/liasmediaplayer" \
  --profile performance --trust-native-code
```

This compiles the mod into Horizon and selects the resulting build for the
launcher's selected instance. Start that instance, enable **Lia's Media Player**
under **Options > Mods**, and enter a world. `--trust-native-code` acknowledges
that the compiled mod runs with your user account's permissions.

To create the distributable source bundle instead, install the registry validator
as described in the [build guide](docs/BUILDING.md), then run:

```sh
python3 tools/package.py
```

The outputs are `liasmediaplayer-4.0.0.hnmod` and its `.sha256` checksum in the
repository root. Packaging does not compile or install the game. The build guide
covers prerequisites, unpacked bundles, profiles, addon builds, validation,
signing, and troubleshooting.

## Using the player

Playback requires the mod to be enabled and a world to be open. Type:

```text
/show <image|gif|video|audio> <url> [newPlayer]
```

For example (replace the example URLs with real media):

```text
/show video https://www.youtube.com/watch?v=VIDEO_ID
/show audio https://example.org/music.mp3 true
/show image https://example.org/picture.png
/show gif https://example.org/animation.gif true
```

The command is handled locally; it does not post the URL to server chat. URLs
must use HTTP or HTTPS. The type is case-insensitive, and URLs may be wrapped in
double quotes. Omit `newPlayer`, or use `false`, to queue into the front compatible
audio/video player or replace the front image. Use `true` for a separate window.
If no compatible window exists, the player creates one.

`/show video` and `/show audio` expand recognized collection URLs. YouTube watch
and short links containing `list=` are treated as playlists; remove `list=` to
play only that video. Queue and collection operations are limited to 500 tracks.
Unplayable private/deleted playlist entries may be skipped by the extractor.

### Controls

| Default key | Action |
| --- | --- |
| `M` | Open Media Controls, a transparent workspace for interacting with players |
| `K` | Play/pause the front player |
| `N` | Next track |
| `P` | Open Playlists |
| `H` | Open Media History |

Rebind these in Horizon's mod key settings under **Options > Mods**. Settings are
also available from the mod's settings page and the media views' navigation.

Open chat to interact with its media badges: click to open, Shift+click to enqueue,
or Ctrl+click to open the original URL in the browser. In a player's queue,
double-click a track to play it, drag to reorder, and use its remove button to
delete it. Repeat cycles **off → all → one → off**. Hover player icons for help;
the **…** menu exposes playback options and error details.

## Tools and saved data

FFmpeg handles audio/video decoding; yt-dlp handles supported website extraction
and collection expansion. Common image formats can be decoded without them.
The tool manager searches the mod's `bin/` directory, the legacy Java-mod `bin/`,
then `PATH` when system-tool lookup is enabled. Missing tools are downloaded on
demand when automatic downloads are enabled. Settings exposes tool status and
update controls. See [runtime tools](docs/BUILDING.md#runtime-media-tools) for
platform details.

The data directory is relative to the active Horizon instance's game directory:

```text
horizon-mods/.horizon-mod-data/liasmediaplayer/
├── settings.json
├── library.json
├── windows.json
├── bin/
└── cache/
```

On first initialization, when the new `settings.json` does not exist, the mod
copies supported files from `<game-directory>/liasmediaplayer/`. To migrate from
a separate Java installation, copy its `liasmediaplayer` folder there before
first use. Legacy v2 JSON is migrated, with backup files retained; unreadable JSON
is preserved as `*.broken-<timestamp>.json`. Existing Rust settings prevent the
automatic first-run copy. Keep a backup before manually replacing saved data.

## Development

```sh
cargo build --workspace --locked
cargo test --workspace --locked
cargo doc -p lias_media_player --no-deps --locked
```

These build, test, and document the independent Rust library. They do not compile
the injected Horizon bridge. See the [build guide](docs/BUILDING.md) for native
integration checks and opt-in network/audio-device tests.

| Path | Purpose |
| --- | --- |
| `mixins/liasmediaplayer/horizon-mixin.toml` | Crate, module, and hook declarations |
| `mixins/liasmediaplayer/bridge.rs` and adjacent helpers | Horizon integration |
| `mixins/liasmediaplayer/lias_media_player/src/api/` | Public addon API |
| `…/src/core/` | Resolution, processes, decoding, playback, library, storage |
| `…/src/ui/`, `…/src/platform/`, `…/src/runtime.rs` | UI, host abstraction, runtime |
| `mixins/liasmediaplayer/lias_media_player/tests/` | Milestone regression suites |
| `examples/addon/` | Addon compiled alongside the player |
| `tools/` | Packaging, packaging tests, command-completion checks, previews |

The Rust API shares concepts with the Java API but does not implement all of its
extension points. See the [Java-to-Rust API comparison](docs/API.md#coming-from-the-java-mod).
The Cargo and compatibility manifests declare the Rust package's license as MIT.
