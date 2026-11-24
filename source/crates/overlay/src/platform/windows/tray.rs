//! Notification-area icon. The icon image is drawn at startup with the
//! overlay's own renderer, so no resource compiler is needed.

use super::{wide, WM_TRAY};
use uo_overlay::core::icon_pixels;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub struct Tray {
    nid: NOTIFYICONDATAW,
    added: bool,
}

fn make_icon(size: i32) -> HICON {
    let px = icon_pixels(size);
    unsafe {
        let mut bi: BITMAPINFO = std::mem::zeroed();
        bi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
        bi.bmiHeader.biWidth = size;
        bi.bmiHeader.biHeight = -size;
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let color = CreateDIBSection(
            std::ptr::null_mut(),
            &bi,
            DIB_RGB_COLORS,
            &mut bits,
            std::ptr::null_mut(),
            0,
        );
        if color.is_null() {
            return std::ptr::null_mut();
        }
        // Icons want straight (non-premultiplied) alpha, which icon_pixels gives.
        std::ptr::copy_nonoverlapping(px.as_ptr(), bits as *mut u32, px.len());
        let mask = CreateBitmap(size, size, 1, 1, std::ptr::null());
        let ii = ICONINFO {
            fIcon: 1,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: color,
        };
        let icon = CreateIconIndirect(&ii);
        DeleteObject(mask);
        DeleteObject(color);
        icon
    }
}

impl Tray {
    pub fn new(hwnd: HWND) -> Self {
        let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
        nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
        nid.hWnd = hwnd;
        nid.uID = 1;
        nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
        nid.uCallbackMessage = WM_TRAY;
        let size = unsafe { GetSystemMetrics(SM_CXSMICON) }.max(16);
        nid.hIcon = make_icon(size);
        let mut t = Self { nid, added: false };
        t.set_tip_raw("Universal Overlay");
        t
    }

    fn set_tip_raw(&mut self, tip: &str) {
        let w = wide(tip);
        let n = w.len().min(self.nid.szTip.len() - 1);
        self.nid.szTip[..n].copy_from_slice(&w[..n]);
        self.nid.szTip[n] = 0;
    }

    pub fn set_tip(&mut self, tip: &str) {
        self.set_tip_raw(tip);
        if self.added {
            unsafe { Shell_NotifyIconW(NIM_MODIFY, &self.nid) };
        }
    }

    pub fn add(&mut self) {
        self.added = unsafe { Shell_NotifyIconW(NIM_ADD, &self.nid) } != 0;
    }

    pub fn remove(&mut self) {
        if self.added {
            unsafe { Shell_NotifyIconW(NIM_DELETE, &self.nid) };
            self.added = false;
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        self.remove();
        unsafe { DestroyIcon(self.nid.hIcon) };
    }
}
