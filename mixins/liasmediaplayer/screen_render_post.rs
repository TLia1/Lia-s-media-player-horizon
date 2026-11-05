// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_screen_render_post(
    game: &crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    screen: &crate::game::native_hooks::NativeScreenView<'_>,
) {
    crate::game::liasmediaplayer_bridge::on_screen_render(game, batch, screen);
}
