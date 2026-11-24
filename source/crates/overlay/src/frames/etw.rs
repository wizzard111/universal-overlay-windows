//! Frame timing without injecting into the game.
//!
//! We open a real-time ETW session and listen to the "Present" events that
//! the graphics runtimes emit for every frame (the same source Intel
//! PresentMon uses):
//!   * Microsoft-Windows-DXGI     Present_Start (42) / PresentMPO_Start (55)  — D3D10/11/12
//!   * Microsoft-Windows-D3D9     Present_Start (1)                           — D3D9
//!   * Microsoft-Windows-DxgKrnl  Present_Info  (184)                         — everything that
//!     reaches the kernel, which covers Vulkan and OpenGL
//!
//! Timestamps are raw QPC ticks (sub-microsecond resolution), so a frame
//! time is simply the delta between two consecutive presents of the same
//! swap chain — PresentMon's "MsBetweenPresents".
//!
//! Only events from the target process (the focused window's process) are
//! stored, so the callback is a pid compare for everything else.
//! A real-time ETW session needs Administrator rights (or membership in
//! "Performance Log Users").

use super::{best_stream, Frame, Status, Stream};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;
use windows_sys::core::GUID;
use windows_sys::Win32::System::Diagnostics::Etw::*;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

const DXGI: GUID = GUID::from_u128(0xCA11C036_0102_4A2D_A6AD_F03CFED5D3C9);
const D3D9: GUID = GUID::from_u128(0x783ACA0A_790E_4D7F_8451_AA850511C6B9);
const DXGKRNL: GUID = GUID::from_u128(0x802EC45A_1E99_4B83_9920_87C98277BA9D);

const SRC_DXGI: u8 = 0;
const SRC_D3D9: u8 = 1;
const SRC_KRNL: u8 = 2;

const SESSION_NAME: &str = "UniversalOverlay-Present";
/// LogFileMode flag: FlushTimer is in milliseconds (Windows 8+).
const EVENT_TRACE_USE_MS_FLUSH_TIMER: u32 = 0x10;
/// How often ETW hands buffers to us. Lower = less graph latency.
pub const FLUSH_MS: u32 = 100;
/// Worst-case delay between a present and us seeing it.
pub const LATENCY_S: f64 = FLUSH_MS as f64 * 1.5 / 1000.0;

struct Store {
    pid: u32,
    streams: HashMap<(u8, u64), Stream>,
    keep_qpc: i64,
    qpf: f64,
}

static TARGET: AtomicU32 = AtomicU32::new(0);
static STORE: Mutex<Option<Store>> = Mutex::new(None);

fn store() -> std::sync::MutexGuard<'static, Option<Store>> {
    STORE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now() -> i64 {
    let mut t = 0i64;
    unsafe { QueryPerformanceCounter(&mut t) };
    t
}

pub fn freq() -> f64 {
    let mut f = 0i64;
    unsafe { QueryPerformanceFrequency(&mut f) };
    f as f64
}

/// Focus the measurement on one process. Clears history on change.
pub fn set_target(pid: u32) {
    if TARGET.swap(pid, Ordering::Relaxed) != pid {
        if let Some(st) = store().as_mut() {
            st.pid = pid;
            st.streams.clear();
        }
    }
}

pub fn set_history_seconds(s: f64) {
    if let Some(st) = store().as_mut() {
        st.keep_qpc = (s * st.qpf) as i64;
    }
}

fn on_present(pid: u32, src: u8, chain: u64, ts: i64) {
    let mut g = store();
    let Some(st) = g.as_mut() else { return };
    if st.pid != pid {
        return;
    }
    let qpf = st.qpf;
    let keep = st.keep_qpc;
    if st.streams.len() > 32 {
        st.streams
            .retain(|_, s| ts - s.last_ts < (5.0 * qpf) as i64);
    }
    // DXGI/D3D9 streams outrank the kernel stream, which duplicates them.
    let prio = if src == SRC_KRNL { 0 } else { 1 };
    st.streams
        .entry((src, chain))
        .or_insert_with(|| Stream::new(prio))
        .push(ts, qpf, keep);
}

unsafe extern "system" fn on_event(rec: *mut EVENT_RECORD) {
    let r = &*rec;
    let h = &r.EventHeader;
    let pid = h.ProcessId;
    if pid == 0 || pid != TARGET.load(Ordering::Relaxed) {
        return;
    }
    let id = h.EventDescriptor.Id;
    let p = h.ProviderId;
    let src = if p.data1 == DXGI.data1 && (id == 42 || id == 55) {
        SRC_DXGI
    } else if p.data1 == D3D9.data1 && id == 1 {
        SRC_D3D9
    } else if p.data1 == DXGKRNL.data1 && id == 184 {
        SRC_KRNL
    } else {
        return;
    };
    let ptr_size = if (h.Flags as u32) & EVENT_HEADER_FLAG_32_BIT_HEADER != 0 {
        4
    } else {
        8
    };
    let data = r.UserData as *const u8;
    let len = r.UserDataLength as usize;
    let mut chain = 0u64;
    if src != SRC_KRNL && !data.is_null() && len >= ptr_size {
        chain = if ptr_size == 8 {
            (data as *const u64).read_unaligned()
        } else {
            (data as *const u32).read_unaligned() as u64
        };
        // DXGI_PRESENT_TEST (0x1) presents do not produce a frame.
        if src == SRC_DXGI && len >= ptr_size + 4 {
            let flags = (data.add(ptr_size) as *const u32).read_unaligned();
            if flags & 1 != 0 {
                return;
            }
        }
    }
    on_present(pid, src, chain, h.TimeStamp);
}

#[repr(C)]
struct Props {
    p: EVENT_TRACE_PROPERTIES,
    name: [u16; 64],
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn new_props() -> Box<Props> {
    let mut b: Box<Props> = Box::new(unsafe { std::mem::zeroed() });
    b.p.Wnode.BufferSize = std::mem::size_of::<Props>() as u32;
    b.p.Wnode.Flags = WNODE_FLAG_TRACED_GUID;
    b.p.Wnode.ClientContext = 1; // QPC timestamps
    b.p.LogFileMode = EVENT_TRACE_REAL_TIME_MODE | EVENT_TRACE_USE_MS_FLUSH_TIMER;
    b.p.FlushTimer = FLUSH_MS;
    b.p.BufferSize = 16; // KB
    b.p.MinimumBuffers = 4;
    b.p.MaximumBuffers = 32;
    b.p.LoggerNameOffset = std::mem::offset_of!(Props, name) as u32;
    b
}

fn stop_by_name() {
    let name = wide(SESSION_NAME);
    let mut props = new_props();
    unsafe {
        ControlTraceW(
            CONTROLTRACE_HANDLE { Value: 0 },
            name.as_ptr(),
            &mut props.p,
            EVENT_TRACE_CONTROL_STOP,
        );
    }
}

#[repr(C)]
struct EventIdFilter {
    filter_in: u8,
    reserved: u8,
    count: u16,
    events: [u16; 4],
}

fn enable(h: CONTROLTRACE_HANDLE, guid: &GUID, keywords: u64, ids: &[u16]) -> u32 {
    let mut f = EventIdFilter {
        filter_in: 1,
        reserved: 0,
        count: ids.len() as u16,
        events: [0; 4],
    };
    f.events[..ids.len()].copy_from_slice(ids);
    let mut desc = EVENT_FILTER_DESCRIPTOR {
        Ptr: &f as *const _ as u64,
        Size: (4 + 2 * ids.len()) as u32,
        Type: EVENT_FILTER_TYPE_EVENT_ID,
    };
    let mut params: ENABLE_TRACE_PARAMETERS = unsafe { std::mem::zeroed() };
    params.Version = ENABLE_TRACE_PARAMETERS_VERSION_2;
    params.EnableFilterDesc = &mut desc;
    params.FilterDescCount = 1;
    unsafe {
        let r = EnableTraceEx2(
            h,
            guid,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER,
            TRACE_LEVEL_VERBOSE as u8,
            keywords,
            0,
            0,
            &params,
        );
        if r == 0 {
            return 0;
        }
        // Older systems: no event-id filter; the callback filters anyway.
        EnableTraceEx2(
            h,
            guid,
            EVENT_CONTROL_CODE_ENABLE_PROVIDER,
            TRACE_LEVEL_VERBOSE as u8,
            keywords,
            0,
            0,
            std::ptr::null(),
        )
    }
}

pub struct Session {
    handle: CONTROLTRACE_HANDLE,
    thread: Option<JoinHandle<()>>,
}

impl Session {
    pub fn start(history_s: f64) -> Result<Session, Status> {
        let qpf = freq();
        *store() = Some(Store {
            pid: TARGET.load(Ordering::Relaxed),
            streams: HashMap::new(),
            keep_qpc: (history_s * qpf) as i64,
            qpf,
        });
        stop_by_name(); // a session left over from a crash
        let name = wide(SESSION_NAME);
        let mut props = new_props();
        let mut handle = CONTROLTRACE_HANDLE { Value: 0 };
        let r = unsafe { StartTraceW(&mut handle, name.as_ptr(), &mut props.p) };
        if r == 5 {
            return Err(Status::AccessDenied);
        }
        if r != 0 {
            return Err(Status::Error(r));
        }
        // Keywords: DXGI/D3D9 "Present" task keyword 0x2; DxgKrnl "Present" 0x8000000.
        enable(handle, &DXGI, 0x2, &[42, 55]);
        enable(handle, &D3D9, 0x2, &[1]);
        enable(handle, &DXGKRNL, 0x800_0000, &[184]);

        let mut lf: EVENT_TRACE_LOGFILEW = unsafe { std::mem::zeroed() };
        let mut name_buf = wide(SESSION_NAME);
        lf.LoggerName = name_buf.as_mut_ptr();
        lf.Anonymous1.ProcessTraceMode = PROCESS_TRACE_MODE_REAL_TIME
            | PROCESS_TRACE_MODE_EVENT_RECORD
            | PROCESS_TRACE_MODE_RAW_TIMESTAMP;
        lf.Anonymous2.EventRecordCallback = Some(on_event);
        let th = unsafe { OpenTraceW(&mut lf) };
        if th.Value == u64::MAX {
            let mut s = Session {
                handle,
                thread: None,
            };
            s.stop_inner();
            return Err(Status::Error(unsafe {
                windows_sys::Win32::Foundation::GetLastError()
            }));
        }
        let thread = std::thread::Builder::new()
            .name("etw-present".into())
            .spawn(move || unsafe {
                let _keep = name_buf;
                ProcessTrace(&th, 1, std::ptr::null(), std::ptr::null());
                CloseTrace(th);
            })
            .ok();
        Ok(Session { handle, thread })
    }

    fn stop_inner(&mut self) {
        let mut props = new_props();
        unsafe {
            ControlTraceW(
                self.handle,
                std::ptr::null(),
                &mut props.p,
                EVENT_TRACE_CONTROL_STOP,
            );
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop_inner();
        *store() = None;
    }
}

// ---------------------------------------------------------------- reading

/// Frames of the most relevant swap chain of the target process.
/// DXGI/D3D9 streams win over the kernel stream (which duplicates them);
/// among equals the busiest stream in the last second wins.
pub fn snapshot(t: super::Tail) -> Option<Vec<Frame>> {
    let g = store();
    let st = g.as_ref()?;
    let best = best_stream(st.streams.values(), now(), st.qpf)?;
    Some(super::tail(&best.frames, st.qpf, t))
}
