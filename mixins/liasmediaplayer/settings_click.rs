// Handles button clicks on Options > Mods > Lia's Media Player (`native_settings_click`).
fn liasmediaplayer_settings_click(
    game: &mut crate::game::Game<'_>,
    mod_id: &str,
    button: usize,
) {
    crate::game::liasmediaplayer_bridge::on_settings_click(game, mod_id, button);
}
