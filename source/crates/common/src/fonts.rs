//! Installed font discovery. On Windows the font registry is used
//! (family name -> file); elsewhere the usual font directories are scanned.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct FontEntry {
    pub name: String,
    pub path: PathBuf,
}

fn supported(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("ttf" | "otf" | "ttc")
    )
}

/// All installed fonts usable by the overlay, sorted by name.
pub fn list_fonts() -> Vec<FontEntry> {
    let mut v = platform::list();
    v.retain(|f| supported(&f.path));
    v.sort_by_cached_key(|f| f.name.to_lowercase());
    v.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
    v
}

/// Resolve a family name or a file path to a font file.
pub fn resolve(name_or_path: &str) -> Option<PathBuf> {
    let p = Path::new(name_or_path);
    if p.is_file() {
        return Some(p.to_path_buf());
    }
    if let Some(f) = list_fonts()
        .into_iter()
        .find(|f| f.name.eq_ignore_ascii_case(name_or_path.trim()))
    {
        return Some(f.path);
    }
    None
}

mod platform {
    use super::FontEntry;
    use std::path::PathBuf;
    use windows_sys::Win32::System::Registry::*;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn list() -> Vec<FontEntry> {
        let fonts_dir = std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:\\Windows"))
            .join("Fonts");
        let mut out = Vec::new();
        for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            read_key(root, &fonts_dir, &mut out);
        }
        out
    }

    fn read_key(root: HKEY, fonts_dir: &std::path::Path, out: &mut Vec<FontEntry>) {
        let sub = wide("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts");
        let mut key: HKEY = std::ptr::null_mut();
        unsafe {
            if RegOpenKeyExW(root, sub.as_ptr(), 0, KEY_READ, &mut key) != 0 {
                return;
            }
            let mut i = 0u32;
            loop {
                let mut name = [0u16; 512];
                let mut name_len = name.len() as u32;
                let mut data = [0u16; 1024];
                let mut data_len = (data.len() * 2) as u32;
                let mut ty = 0u32;
                let r = RegEnumValueW(
                    key,
                    i,
                    name.as_mut_ptr(),
                    &mut name_len,
                    std::ptr::null(),
                    &mut ty,
                    data.as_mut_ptr() as *mut u8,
                    &mut data_len,
                );
                i += 1;
                if r == 259 {
                    break; // ERROR_NO_MORE_ITEMS
                }
                if r != 0 || ty != REG_SZ {
                    continue;
                }
                let name = String::from_utf16_lossy(&name[..name_len as usize]);
                let file = String::from_utf16_lossy(&data[..(data_len as usize / 2)]);
                let file = file.trim_end_matches('\0');
                let path = if file.contains('\\') {
                    PathBuf::from(file)
                } else {
                    fonts_dir.join(file)
                };
                // "Segoe UI Bold (TrueType)" -> "Segoe UI Bold";
                // "Cambria & Cambria Math (TrueType)" -> two names.
                let base = match name.rfind(" (") {
                    Some(p) => &name[..p],
                    None => name.as_str(),
                };
                for n in base.split(" & ") {
                    out.push(FontEntry {
                        name: n.trim().to_string(),
                        path: path.clone(),
                    });
                }
            }
            RegCloseKey(key);
        }
    }
}
