//! Protocol between `uo-sensord` (a tiny Windows service that reads the CPU
//! temperature through the PawnIO driver) and the overlay, which runs as a
//! normal user and never needs administrator rights.
//!
//! The service owns a named shared-memory section and two auto-reset events:
//!   * the overlay sets `WANT` (once per sensor tick, only while visible),
//!   * the service reads the sensor (one DeviceIoControl, microseconds),
//!     stores the result and sets `DONE`,
//!   * the overlay waits for `DONE` (a few ms at most) and reads the values.
//!
//! While the overlay is hidden nobody sets `WANT` and the service sleeps in
//! the kernel: zero CPU.

use std::sync::atomic::{AtomicI32, AtomicU32};

pub const SERVICE_NAME: &str = "UniversalOverlaySensors";
pub const SERVICE_DISPLAY: &str = "Universal Overlay — датчики";
pub const SERVICE_EXE: &str = "uo-sensord.exe";

pub const MAPPING: &str = "Global\\UniversalOverlay.Sensors";
pub const EVENT_WANT: &str = "Global\\UniversalOverlay.SensorsWant";
pub const EVENT_DONE: &str = "Global\\UniversalOverlay.SensorsDone";

pub const MAGIC: u32 = 0x554F_5344; // "UOSD"

/// Result codes in [`Shared::status`].
pub const STATUS_OK: i32 = 0;
pub const STATUS_UNSUPPORTED_CPU: i32 = 1;
pub const STATUS_NO_DRIVER: i32 = 2;
pub const STATUS_NO_MODULE: i32 = 3;
pub const STATUS_FAILED: i32 = 4;

/// Layout of the shared section. Floats are stored as their bit patterns
/// so every field is a plain atomic.
#[repr(C)]
pub struct Shared {
    pub magic: AtomicU32,
    /// Incremented after every reading.
    pub seq: AtomicU32,
    pub status: AtomicI32,
    /// CPU package / Tctl temperature, °C (f32 bits; NaN = no value).
    pub cpu_temp: AtomicU32,
    /// AMD: hottest CCD, °C (f32 bits; NaN = no value).
    pub ccd_max: AtomicU32,
    pub _reserved: [u32; 3],
}

pub const SIZE: usize = std::mem::size_of::<Shared>();

pub fn status_text(s: i32) -> &'static str {
    match s {
        STATUS_OK => "ok",
        STATUS_UNSUPPORTED_CPU => "процессор не поддерживается",
        STATUS_NO_DRIVER => "драйвер PawnIO не установлен",
        STATUS_NO_MODULE => "нет модуля PawnIO",
        _ => "ошибка драйвера",
    }
}
