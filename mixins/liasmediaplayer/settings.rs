// Populates buttons on Options > Mods > Lia's Media Player (`native_settings`).
fn liasmediaplayer_settings(
    game: &mut crate::game::Game<'_>,
    mod_id: &str,
    buttons: &mut Vec<String>,
) {
    crate::game::liasmediaplayer_bridge::on_settings(game, mod_id, buttons);
}
