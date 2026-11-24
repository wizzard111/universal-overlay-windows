include!("../../resources/winres.rs");

fn main() {
    embed_resources(
        "Universal Overlay - sensor service",
        "uo-sensord.exe",
        Level::AsInvoker,
    );
}
