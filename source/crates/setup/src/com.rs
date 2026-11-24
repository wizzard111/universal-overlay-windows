//! Start-menu shortcuts through the Shell's own COM object (ShellLink),
//! with hand-written vtables — no big binding crate needed for two calls.

use std::ffi::c_void;
use std::path::Path;
use windows_sys::core::GUID;
use windows_sys::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);
const IID_ISHELL_LINK_W: GUID = GUID::from_u128(0x000214f9_0000_0000_c000_000000000046);
const IID_IPERSIST_FILE: GUID = GUID::from_u128(0x0000010b_0000_0000_c000_000000000046);

type Hr = i32;
type This = *mut c_void;

/// IShellLinkW vtable (IUnknown + IShellLinkW methods, in declaration order).
#[repr(C)]
struct ShellLinkVtbl {
    query_interface: unsafe extern "system" fn(This, *const GUID, *mut This) -> Hr,
    add_ref: unsafe extern "system" fn(This) -> u32,
    release: unsafe extern "system" fn(This) -> u32,
    get_path: usize,
    get_id_list: usize,
    set_id_list: usize,
    get_description: usize,
    set_description: unsafe extern "system" fn(This, *const u16) -> Hr,
    get_working_directory: usize,
    set_working_directory: unsafe extern "system" fn(This, *const u16) -> Hr,
    get_arguments: usize,
    set_arguments: unsafe extern "system" fn(This, *const u16) -> Hr,
    get_hotkey: usize,
    set_hotkey: usize,
    get_show_cmd: usize,
    set_show_cmd: usize,
    get_icon_location: usize,
    set_icon_location: unsafe extern "system" fn(This, *const u16, i32) -> Hr,
    set_relative_path: usize,
    resolve: usize,
    set_path: unsafe extern "system" fn(This, *const u16) -> Hr,
}

/// IPersistFile vtable (IUnknown + IPersist + IPersistFile).
#[repr(C)]
struct PersistFileVtbl {
    query_interface: usize,
    add_ref: usize,
    release: unsafe extern "system" fn(This) -> u32,
    get_class_id: usize,
    is_dirty: usize,
    load: usize,
    save: unsafe extern "system" fn(This, *const u16, i32) -> Hr,
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn check(hr: Hr, what: &str) -> Result<(), String> {
    if hr < 0 {
        Err(format!("{what}: 0x{hr:08X}"))
    } else {
        Ok(())
    }
}

/// Create (or overwrite) a .lnk pointing at `target`.
pub fn shortcut(
    lnk: &Path,
    target: &Path,
    args: &str,
    workdir: &Path,
    description: &str,
) -> Result<(), String> {
    unsafe {
        CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let mut link: This = std::ptr::null_mut();
        check(
            CoCreateInstance(
                &CLSID_SHELL_LINK,
                std::ptr::null_mut(),
                CLSCTX_INPROC_SERVER,
                &IID_ISHELL_LINK_W,
                &mut link,
            ),
            "ShellLink",
        )?;
        let vt = &**(link as *mut *const ShellLinkVtbl);
        let target_w = wide(&target.to_string_lossy());
        let result = (|| {
            check((vt.set_path)(link, target_w.as_ptr()), "SetPath")?;
            check(
                (vt.set_arguments)(link, wide(args).as_ptr()),
                "SetArguments",
            )?;
            check(
                (vt.set_working_directory)(link, wide(&workdir.to_string_lossy()).as_ptr()),
                "SetWorkingDirectory",
            )?;
            check(
                (vt.set_description)(link, wide(description).as_ptr()),
                "SetDescription",
            )?;
            check(
                (vt.set_icon_location)(link, target_w.as_ptr(), 0),
                "SetIconLocation",
            )?;
            let mut pf: This = std::ptr::null_mut();
            check(
                (vt.query_interface)(link, &IID_IPERSIST_FILE, &mut pf),
                "IPersistFile",
            )?;
            let pvt = &**(pf as *mut *const PersistFileVtbl);
            let r = check(
                (pvt.save)(pf, wide(&lnk.to_string_lossy()).as_ptr(), 1),
                "Save",
            );
            (pvt.release)(pf);
            r
        })();
        (vt.release)(link);
        let _ = vt.add_ref; // keep the vtable layout explicit
        result
    }
}
