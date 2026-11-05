// Catalog hook adapter; the crate owns all UI behavior.
fn liasmediaplayer_client_resources_reloaded() {
    crate::game::liasmediaplayer_bridge::on_resources_reloaded();
}
