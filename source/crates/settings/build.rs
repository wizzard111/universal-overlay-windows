include!("../../resources/winres.rs");

fn main() {
    embed_resources(
        "Universal Overlay - settings",
        "universal-overlay-settings.exe",
        Level::AsInvoker,
    );
}
