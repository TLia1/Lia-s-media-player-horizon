# Building Lia's Media Player

Run repository commands from the repository root unless a step says otherwise.
The shell examples use Bash on Linux. The manifests, packaging script, and
installed launcher's `native-build --help` are the source of truth for this build.

## What gets built

| Operation | Output | Requires a Horizon instance? |
| --- | --- | --- |
| `cargo build --workspace --locked` | Independent Rust library under `target/debug/` | No |
| `cargo test --workspace --locked` | Library and integration-test results | No |
| `cargo doc -p lias_media_player --no-deps --locked` | HTML under `target/doc/lias_media_player/` | No |
| `python3 tools/package.py` | Validated `.hnmod` source archive and checksum | No |
| `horizon-launcher native-build …` | Horizon executable containing the mod | Yes |

The independent library has no Horizon imports. Hook helpers and `bridge.rs` are
compiled only when woven into Horizon. Passing Cargo tests or archive validation
therefore does not establish that the native client builds.

## Prerequisites

- Git and a working native C/C++ toolchain/linker.
- Rust and Cargo supporting edition 2024 and the locked dependencies. This
  checkout has no `rust-toolchain.toml` or declared minimum Rust version. Local
  documentation validation used Rust `1.99.1`; use the launcher's managed compiler
  for native client builds.
- Python **3.11+** (`tools/package.py` uses standard-library `tomllib`). No pip
  dependencies are needed for packaging or its unit tests.
- On Linux, ALSA development files and `pkg-config` for CPAL; the full Horizon
  client also needs udev development files.
- Horizon Launcher with a prepared Java Edition **26.3** instance for an in-game
  build. Prepare the instance through the launcher before compiling the mod.
- `cargo-horizon-mod` for archive validation and packaging.
- Optional: FFmpeg/ffprobe for media integration tests, yt-dlp for website
  playback, and minisign for signing releases.

For Debian/Ubuntu, the native build dependencies can be installed with:

```sh
sudo apt install build-essential pkg-config libasound2-dev libudev-dev \
  git python3 rsync xz-utils
```

Verify that your distribution's Python is at least 3.11. On Arch Linux, the
corresponding packages include `base-devel`, `alsa-lib`, `systemd`, `git`,
`python`, `rsync`, and `xz`. Use a native platform toolchain on other systems;
the full Windows/macOS build has not been validated by this documentation change.

Install the validator from the declared Horizon revision:

```sh
cargo install --locked --git https://github.com/horizon-rust/horizon \
  --rev 6475f62f3c834b7489e1816bf47f47ea5eb2b879 cargo-horizon-mod
```

Ensure Cargo's installation bin directory is on `PATH` (normally
`~/.cargo/bin`). The official tool's source is
[`crates/cargo-horizon-mod`](https://github.com/horizon-rust/horizon/tree/6475f62f3c834b7489e1816bf47f47ea5eb2b879/crates/cargo-horizon-mod).

## 1. Get the source

```sh
git clone https://github.com/TLia1/Lia-s-media-player-for-horizon.git
cd Lia-s-media-player-for-horizon
```

The workspace has one member:
`mixins/liasmediaplayer/lias_media_player`. Keep `Cargo.lock` and use `--locked`
so local checks do not silently change dependency resolution. An initial Cargo
build needs network access unless the locked crates are already cached.

## 2. Build and check the library

```sh
cargo build --workspace --locked
cargo fmt --all -- --check
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
python3 -m unittest discover -s tools -p 'test_*.py'
cargo doc -p lias_media_player --no-deps --locked
```

Open `target/doc/lias_media_player/index.html` for generated Rust documentation.
Use [API.md](API.md) for the supported addon contract and complete usage guide.
If rustfmt/Clippy are missing from a rustup-managed installation, install them
with `rustup component add rustfmt clippy`.

Tests include local HTTP servers and subprocess fixtures. Loopback networking
and process creation must be available. FFmpeg-dependent cases can skip if their
tools are missing, so install `ffmpeg` and `ffprobe` on `PATH` to exercise them.
The normal suite excludes the two tests below:

```sh
# Contacts the network and downloads a live yt-dlp release.
cargo test --workspace --locked --test milestone3 live_ytdlp_download \
  -- --ignored --nocapture

# Opens the default audio output device and sends silent PCM.
cargo test --workspace --locked --test milestone4 \
  cpal_device_callback_consumes_pcm_and_honors_pause -- --ignored
```

Use `--offline` in addition to `--locked` only after dependencies are cached.
Pillow is needed only to regenerate image fixtures with `tools/image_fixtures.py`;
ordinary tests use checked-in fixtures.

## 3. Compile into Horizon

With the intended instance selected in the launcher:

```sh
horizon-launcher native-build "$PWD/mixins/liasmediaplayer" \
  --profile dev --trust-native-code
```

To target a specific existing instance, add `--instance "INSTANCE_NAME"`. The
launcher downloads the required source/toolchain as needed, applies the mixins,
checks the mod crate against Horizon's dependency lock, compiles, and selects the
new native build for that instance. Press **Play**, enable the mod in
**Options > Mods**, enter a world, and try `/show` and `M`.

| Profile | Intended use |
| --- | --- |
| `dev` | Fast iteration, slower gameplay |
| `performance` | Incremental builds with gameplay optimization |
| `release` | Thin LTO; slowest build, intended for final performance checks |

For regular play, repeat the command with `--profile performance`. A first build
compiles Horizon and its dependencies; later builds can reuse cached work.
This workflow follows Horizon's [native-mod setup guide](https://playhorizon.dev/docs/make-a-mod).

The bundle pins Horizon `6475f62f3c834b7489e1816bf47f47ea5eb2b879`, hooks 359,
and horizon-mixins `e8cd0aa38cffef49cb2e088ef716baa107148c02`. Keep those pins
for the documented target. `--ignore-pins`, `--update`, and
`--allow-version-mismatch` are not needed for this workflow; they change the
compatibility conditions. Check your installed launcher's help before testing a
different revision.

### Optional independent native integration check

The official registry repository provides a checker that applies the bundle and
runs Cargo against Horizon without selecting a launcher build:

```sh
# Start in this repository, then use a separate directory for the registry tools.
LMP_SOURCE="$PWD"
git clone https://github.com/horizon-rust/horizon-mods ../horizon-mods
cd ../horizon-mods
python3 native/tools/native.py check "$LMP_SOURCE"
cd "$LMP_SOURCE"
```

This requires the checker dependencies, including `rsync`. See the official
[native testing guide](https://playhorizon.dev/docs/test-a-mod). The old
`python3 scripts/native.py …` path in the implementation blueprint is not a
script shipped by this repository.

### Command-completion regression check

After a native build, point the checker at that build's Cargo profile `deps/`
directory and the **same compiler** that produced its `.rlib` files:

```sh
python3 tools/check_command_tree.py \
  --deps /absolute/path/to/native-target/profile/deps \
  --rustc /absolute/path/to/native-toolchain/bin/rustc
```

Replace both paths using the native build's output/cache location. The directory
must contain `libhorizon_command-*.rlib` and `liblias_media_player-*.rlib`.
Standalone `target/debug/deps` lacks the Horizon dispatcher dependency. Mixing
compilers can cause incompatible-metadata errors even when the source is correct.

## 4. Package a distributable `.hnmod`

```sh
python3 tools/package.py
cargo horizon-mod check liasmediaplayer-4.0.0.hnmod --json
sha256sum -c liasmediaplayer-4.0.0.hnmod.sha256
```

`package.py` itself runs the registry validator before replacing an existing
archive. The explicit second command is useful when checking a received bundle.
The archive contains `horizon-native.json` and the `.rs`/`.toml` sources under
`mixins/liasmediaplayer/`. It excludes tests, fixtures, the example addon, docs,
root assets, Cargo build outputs, and the root workspace manifest/lockfile.
Native dependency resolution uses Horizon's lockfile.

The script checks manifest consistency, rejects source-tree symlinks and
unexpected file types, enforces 512 files / 64 MiB uncompressed, and fixes ZIP
timestamps and permissions for reproducible packaging. Byte-identical inputs in
the same packaging environment produce the same archive. Archive validation
does not compile the source or publish a registry release.

To build a downloaded archive, extract it first; `native-build` expects a folder:

```sh
python3 -m zipfile -e liasmediaplayer-4.0.0.hnmod /tmp/lmp-unpacked
horizon-launcher native-build /tmp/lmp-unpacked \
  --profile performance --trust-native-code
```

Use a new or empty extraction directory to avoid including stale files.
Distribute the `.hnmod` with its `.sha256`, and `.minisig` when signed.

## 5. Optional signing

Generate your own key once, keeping the private key outside the repository:

```sh
mkdir -p "$HOME/.minisign"
minisign -G -s "$HOME/.minisign/my-lmp.key" -p "$HOME/.minisign/my-lmp.pub"
```

Then package and sign with the matching pair:

```sh
python3 tools/package.py \
  --sign-key "$HOME/.minisign/my-lmp.key" \
  --public-key "$HOME/.minisign/my-lmp.pub"
minisign -Vm liasmediaplayer-4.0.0.hnmod -p "$HOME/.minisign/my-lmp.pub"
```

The script verifies the signature using both minisign and the registry validator.
Both key options are required together. Without them, it packages without
creating a signature; if the archive changes, it removes an obsolete `.minisig`.
An unchanged archive may retain its existing signature.

`docs/horizon-mod.pub` is the checked-in release verification key. It can verify
only a release signed by its matching private key, not a build you sign with your
own key. No private key is provided or required for an unsigned local build.
Publication is a separate process described by Horizon's
[publishing guide](https://playhorizon.dev/docs/publish-a-mod).

## Build the example addon with the player

The pinned weaver exposes the player crate to modules in `horizon-client`.
Both native mods must be supplied in the same build. From this repository:

```sh
LMP_ADDON_STAGE="$(mktemp -d)"
cp -R mixins/liasmediaplayer "$LMP_ADDON_STAGE/liasmediaplayer"
cp -R examples/addon "$LMP_ADDON_STAGE/example_radio"
horizon-launcher native-build "$LMP_ADDON_STAGE" \
  --profile dev --trust-native-code
```

This stage is a snapshot; copy changed sources again before rebuilding. The
example registers through the `mods` hook, guarded by `Once`. Its **Radio Violet**
theme should appear in Settings after registration. Its media URLs are reserved
`example.org` placeholders; replace them to test playback.

Do not add a second `lias_media_player` crate or use `[api]` / mod
`[dependencies]` manifest tables: the pinned weaver does not support that addon
loading scheme. See [the API guide](API.md) for registration and lifecycle details.

## Runtime media tools

Runtime playback dependencies are separate from compilation. Install `ffmpeg`,
`ffprobe`, and `yt-dlp` on `PATH`, or allow the mod's automatic downloader.
The downloader selects yt-dlp for Linux x86_64/aarch64, Windows x86_64/aarch64,
and macOS x86_64/aarch64. Its FFmpeg download mapping covers Linux x86_64/aarch64,
Windows x86_64, and macOS x86_64. Other targets must provide a native FFmpeg.
These are code paths, not a claim that every platform has been smoke-tested.
Linux `.tar.xz` extraction needs `tar` with xz support.

The tool search order is the native data directory's `bin/`, legacy
`<game-directory>/liasmediaplayer/bin/`, then `PATH` when allowed. Turning off
automatic downloads requires supplying working tools yourself. Tool errors are
shown in the player and full diagnostics in the game log.

## Troubleshooting

| Symptom | Check |
| --- | --- |
| `alsa-sys` or `pkg-config` fails | Install ALSA development files and `pkg-config`; for the native client also install udev development files. |
| `No module named tomllib` | Use Python 3.11 or later. |
| `no such command: horizon-mod` | Install `cargo-horizon-mod` and add Cargo's bin directory to `PATH`. |
| `E_CRATE_DEPENDENCY` during weaving | Mod dependencies must already exist at compatible versions in the target Horizon lockfile. A successful standalone Cargo build is insufficient. |
| Hook/target/binding not found | Compare all manifest pins with the actual Horizon and weaver revisions. |
| No mod in the running game | Play the instance selected by the successful native build; check Options > Mods and enable the mod. |
| `/show` refuses playback | Enter a world, enable the mod, and supply an HTTP(S) URL. |
| Missing tool / extraction failure | Check tool policy, platform support, `PATH`, and `tar`/xz; inspect the full log. |
| No audio | Check the default OS output device, player volume, and CPAL errors in the log. |
| Addon cannot import `lias_media_player` | Supply the player and addon together and inject the addon module into `horizon-client`. |

The nested crate manifest must remain compatible with the weaver: avoid adding
workspace/profile/target sections or path/git dependencies there. The standalone
workspace may resolve dependencies differently from Horizon, so repeat a native
integration build whenever dependencies or hook helpers change.
