//! Thin wrapper over Windows Performance Data Helper counters.
//! English counter paths are used so it works on any Windows language.

use windows_sys::Win32::System::Performance::*;

pub struct Query {
    q: PDH_HQUERY,
}

#[derive(Clone, Copy)]
pub struct Counter(PDH_HCOUNTER);

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

impl Query {
    pub fn new() -> Option<Self> {
        let mut q: PDH_HQUERY = std::ptr::null_mut();
        if unsafe { PdhOpenQueryW(std::ptr::null(), 0, &mut q) } != 0 {
            return None;
        }
        Some(Self { q })
    }

    pub fn add(&self, path: &str) -> Option<Counter> {
        let p = wide(path);
        let mut c: PDH_HCOUNTER = std::ptr::null_mut();
        if unsafe { PdhAddEnglishCounterW(self.q, p.as_ptr(), 0, &mut c) } != 0 {
            return None;
        }
        Some(Counter(c))
    }

    pub fn collect(&self) -> bool {
        unsafe { PdhCollectQueryData(self.q) == 0 }
    }

    pub fn value(&self, c: Counter) -> Option<f64> {
        let mut v: PDH_FMT_COUNTERVALUE = unsafe { std::mem::zeroed() };
        // PDH_FMT_NOCAP100 = 0x8000: don't clamp utility counters to 100.
        let r = unsafe {
            PdhGetFormattedCounterValue(c.0, PDH_FMT_DOUBLE | 0x8000, std::ptr::null_mut(), &mut v)
        };
        if r != 0 || (v.CStatus != 0 && v.CStatus != 1) {
            return None;
        }
        Some(unsafe { v.Anonymous.doubleValue })
    }

    /// All instances of a wildcard counter as (instance name, value).
    pub fn array(&self, c: Counter) -> Vec<(String, f64)> {
        let mut size = 0u32;
        let mut count = 0u32;
        let fmt = PDH_FMT_DOUBLE | 0x8000;
        unsafe {
            let r =
                PdhGetFormattedCounterArrayW(c.0, fmt, &mut size, &mut count, std::ptr::null_mut());
            if r != PDH_MORE_DATA || size == 0 {
                return Vec::new();
            }
            // Buffer must be 8-byte aligned for the f64 values.
            let mut buf = vec![0u64; (size as usize).div_ceil(8)];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            if PdhGetFormattedCounterArrayW(c.0, fmt, &mut size, &mut count, items) != 0 {
                return Vec::new();
            }
            let mut out = Vec::with_capacity(count as usize);
            for i in 0..count as usize {
                let it = &*items.add(i);
                if it.FmtValue.CStatus != 0 && it.FmtValue.CStatus != 1 {
                    continue;
                }
                let mut len = 0;
                while *it.szName.add(len) != 0 {
                    len += 1;
                }
                let name = String::from_utf16_lossy(std::slice::from_raw_parts(it.szName, len));
                out.push((name, it.FmtValue.Anonymous.doubleValue));
            }
            out
        }
    }
}

impl Drop for Query {
    fn drop(&mut self) {
        unsafe { PdhCloseQuery(self.q) };
    }
}
