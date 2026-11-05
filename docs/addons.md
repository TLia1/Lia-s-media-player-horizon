# Addon quick start

The complete reference is [Rust addon API 4](API.md). The API is a Rust source
interface compiled into Horizon with the player, not a Java API or shared-library
ABI. `lias_media_player::api::API_VERSION` is `4`.

1. Copy the three files in [`examples/addon`](../examples/addon/) into your own
   native-mod source folder.
2. Change the mod/module IDs in `horizon-mixin.toml` and the module call in
   `init.rs`; update namespaces and implementation names in `addon.rs`.
3. Implement `MediaPlayerAddon`. Its `id()` is a namespace such as `my_addon`;
   extension IDs are `my_addon:name`. The player namespace is reserved.
4. Register sources, resolvers, themes, or window actions through `AddonContext`.
   Call `register_addon(Box::new(MyAddon))` once through a guarded installer.
5. [Build the player and addon together](BUILDING.md#build-the-example-addon-with-the-player),
   enter a world with the player enabled, and test your extensions.

The pinned hooks API 359 / weaver `e8cd0aa` exposes the player crate to support
modules injected into `horizon-client`. It does not support the blueprint's
`[api]` or mod `[dependencies]` tables. Do not bundle another copy of the player
crate. The example's `radio.example.org` URLs are placeholders and need replacing
before testing playback.

Sources classify links without I/O; resolvers do bounded, cancellable work on
workers. Media controls use session handles and can return busy errors. Window
buttons emit events through bounded subscriptions; consume them outside callbacks
that hold the runtime lock. The [reference](API.md) documents exact signatures,
limits, error behavior, and differences from the Java API.

For source packaging, checksum verification, and optional signing, see the
[build and release instructions](BUILDING.md#4-package-a-distributable-hnmod).
