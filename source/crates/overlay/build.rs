include!("../../resources/winres.rs");

fn main() {
    embed_resources(
        "Universal Overlay - FPS and sensor overlay",
        "universal-overlay.exe",
        Level::AsInvoker,
    );
}
