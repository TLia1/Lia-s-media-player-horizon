// HUD rendering hook (`native_hud`).
fn liasmediaplayer_hud(
    game: &mut crate::game::Game<'_>,
    batch: &mut horizon_client::hud::HudBatch,
    corners: &mut horizon_client::extras_hud::Corners,
) {
    crate::game::liasmediaplayer_bridge::on_hud(game, batch, corners);
}
