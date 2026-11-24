use serde::{Deserialize, Serialize};

macro_rules! sensors {
    ($( $v:ident => $label:literal, $unit:literal, $dec:literal, $tmpl:literal, $desc:literal; )*) => {
        /// Every metric the overlay knows how to show.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(rename_all = "snake_case")]
        pub enum SensorId { $($v),* }

        impl SensorId {
            pub const ALL: &'static [SensorId] = &[$(SensorId::$v),*];
            pub const COUNT: usize = Self::ALL.len();

            pub fn index(self) -> usize { self as usize }

            /// Short label drawn in the overlay (user can override it).
            pub fn default_label(self) -> &'static str { match self { $(SensorId::$v => $label),* } }

            pub fn unit(self) -> &'static str { match self { $(SensorId::$v => $unit),* } }

            pub fn decimals(self) -> usize { match self { $(SensorId::$v => $dec),* } }

            /// Widest typical value; used to reserve column width so the
            /// overlay does not jitter when digits change.
            pub fn template(self) -> &'static str { match self { $(SensorId::$v => $tmpl),* } }

            /// Human readable description for the settings app.
            pub fn description(self) -> &'static str { match self { $(SensorId::$v => $desc),* } }
        }
    };
}

sensors! {
    Fps        => "FPS",      "",    0, "000",      "Frames per second";
    FrameTime  => "FT",       "ms",  1, "00.0",     "Frame time (average over the FPS window)";
    Low1       => "1% LOW",   "",    0, "000",      "1% low FPS";
    Low01      => "0.1% LOW", "",    0, "000",      "0.1% low FPS";
    GpuLoad    => "GPU",      "%",   0, "100",      "GPU load";
    GpuTemp    => "GPU TEMP", "°C",  0, "100",      "GPU temperature";
    GpuHotspot => "GPU HOT",  "°C",  0, "100",      "GPU temperature (hotspot)";
    GpuMemTemp => "VRAM TEMP","°C",  0, "100",      "VRAM temperature";
    GpuClock   => "GPU CLK",  "MHz", 0, "0000",     "GPU clock";
    GpuMemClock=> "MEM CLK",  "MHz", 0, "0000",     "VRAM clock";
    GpuPower   => "GPU PWR",  "W",   0, "000",      "GPU power";
    GpuVoltage => "GPU V",    "mV",  0, "0000",     "GPU voltage";
    GpuFan     => "FAN",      "RPM", 0, "0000",     "GPU fan speed";
    GpuFanPct  => "FAN",      "%",   0, "100",      "GPU fan speed, %";
    GpuVram    => "VRAM",     "MB",  0, "00000",    "VRAM used";
    CpuLoad    => "CPU",      "%",   0, "100",      "CPU load";
    CpuClock   => "CPU CLK",  "MHz", 0, "0000",     "CPU clock";
    CpuTemp    => "CPU TEMP", "°C",  0, "100",      "CPU temperature";
    RamUsed    => "RAM",      "MB",  0, "00000",    "RAM used";
    RamLoad    => "RAM",      "%",   0, "100",      "RAM used, %";
    Clock      => "TIME",     "",    0, "00:00:00", "Current time";
}
