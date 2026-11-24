//! Diagnostic console tool: prints every sensor the overlay can read and
//! live FPS of a process.
//!
//!   uo-probe            sensors + 10 s of FPS for the foreground window
//!                       (Linux/macOS: for whichever process presents)
//!   uo-probe <pid>      FPS for a given process id
//!   uo-probe --render out.png [secs]
//!                       collect sensors for a few seconds and save a
//!                       picture of the overlay as configured

use uo_common::config::Stats;
use uo_common::sensor::SensorId;
use uo_overlay::{frames, sensors};

fn foreground_pid() -> u32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetWindowThreadProcessId,
    };
    let mut pid = 0;
    unsafe { GetWindowThreadProcessId(GetForegroundWindow(), &mut pid) };
    pid
}

/// Render the overlay (current config, live sensors) to a PNG.
fn render(path: &str, secs: u64) {
    let mut core = uo_overlay::core::Core::new();
    core.start_frames();
    for _ in 0..secs * 4 {
        core.tick(None);
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let mon = uo_overlay::core::Rect {
        x: 0,
        y: 0,
        w: 1920,
        h: 1080,
    };
    core.render(mon, 1.0, 1.0);
    let cv = &core.canvas;
    let straight: Vec<u32> = cv
        .px
        .iter()
        .map(|&p| {
            let a = p >> 24;
            if a == 0 {
                return 0;
            }
            let un = |c: u32| ((c * 255 + a / 2) / a).min(255);
            (a << 24) | (un((p >> 16) & 255) << 16) | (un((p >> 8) & 255) << 8) | un(p & 255)
        })
        .collect();
    match std::fs::write(path, uo_overlay::icon::encode_png(cv.w, cv.h, &straight)) {
        Ok(()) => println!("{path}: {}×{}", cv.w, cv.h),
        Err(e) => eprintln!("{path}: {e}"),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--render") {
        let path = args.get(i + 1).map(String::as_str).unwrap_or("overlay.png");
        render(
            path,
            args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(5),
        );
        return;
    }
    let mut s = sensors::Sensors::new();
    println!("GPU source: {}", s.gpu_name());
    println!("CPU temp source: {}", s.cpu_temp_status());

    if let Some(mut adl) = sensors::windows::gpu_amd::Adl::new() {
        println!("\nADL raw PM-log sensors (index = value):");
        for (i, v) in adl.raw() {
            print!("  [{i:3}]={v:<6}");
        }
        println!();
    }

    let mut snap = sensors::Snapshot::default();
    s.poll(&mut snap, sensors::Want::ALL);
    std::thread::sleep(std::time::Duration::from_millis(1000));
    s.poll(&mut snap, sensors::Want::ALL);
    println!("\nSensors:");
    for id in SensorId::ALL {
        if let Some(v) = snap.get(*id) {
            println!("  {:<10} {:>10.1} {}", id.default_label(), v, id.unit());
        }
    }

    let fixed_pid: Option<u32> = std::env::args().nth(1).and_then(|a| a.parse().ok());
    println!("\nFrame timing...");
    let freq = frames::freq();
    let session = match frames::Session::start(12.0) {
        Ok(s) => Some(s),
        Err(e) => {
            println!("  backend status: {e:?}");
            None
        }
    };
    for _ in 0..10 {
        let pid = fixed_pid.unwrap_or_else(foreground_pid);
        frames::set_target(pid);
        std::thread::sleep(std::time::Duration::from_millis(1000));
        match frames::snapshot(frames::Tail::secs(12.0)) {
            Some(f) => {
                let st = frames::compute_stats(&f, freq, &Stats::default());
                println!(
                    "  pid {pid:6}: fps {:7.2}  ft {:6.2} ms  1% {:6.1}  frames {}",
                    st.fps.unwrap_or(0.0),
                    st.frame_time.unwrap_or(0.0),
                    st.low1.unwrap_or(0.0),
                    f.len()
                );
            }
            None => println!("  pid {pid:6}: no presents"),
        }
    }
    drop(session);
}
