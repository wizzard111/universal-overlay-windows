//! Universal Overlay — entry point; the platform host lives in `platform/`.
#![windows_subsystem = "windows"]

mod platform;

fn main() {
    if uo_overlay::icon::export_from_args() {
        return;
    }
    platform::run();
}
