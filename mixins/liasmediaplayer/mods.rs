// Registers Lia's Media Player in Options > Mods (`native_mods`).
fn liasmediaplayer_mods(mods: &mut Vec<crate::game::native_hooks::NativeMod>) {
    crate::game::liasmediaplayer_bridge::on_mods(mods);
}
