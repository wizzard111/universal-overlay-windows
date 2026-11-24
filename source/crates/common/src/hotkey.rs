//! Hotkey strings like "Ctrl+Shift+O". Key codes are Windows virtual-key
//! codes; other platforms map the canonical names themselves.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hotkey {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub win: bool,
    /// Canonical key name, e.g. "O", "F10", "Insert".
    pub key: String,
}

const NAMED: &[(&str, u32, &[&str])] = &[
    ("Space", 0x20, &["Spacebar"]),
    ("Enter", 0x0D, &["Return"]),
    ("Tab", 0x09, &[]),
    ("Backspace", 0x08, &[]),
    ("Escape", 0x1B, &["Esc"]),
    ("Insert", 0x2D, &["Ins"]),
    ("Delete", 0x2E, &["Del"]),
    ("Home", 0x24, &[]),
    ("End", 0x23, &[]),
    ("PageUp", 0x21, &["PgUp"]),
    ("PageDown", 0x22, &["PgDn"]),
    ("Up", 0x26, &["ArrowUp"]),
    ("Down", 0x28, &["ArrowDown"]),
    ("Left", 0x25, &["ArrowLeft"]),
    ("Right", 0x27, &["ArrowRight"]),
    ("Pause", 0x13, &["Break"]),
    ("ScrollLock", 0x91, &[]),
    ("PrintScreen", 0x2C, &["PrtSc"]),
    ("NumpadMultiply", 0x6A, &[]),
    ("NumpadAdd", 0x6B, &[]),
    ("NumpadSubtract", 0x6D, &[]),
    ("NumpadDecimal", 0x6E, &[]),
    ("NumpadDivide", 0x6F, &[]),
    ("Minus", 0xBD, &["-"]),
    ("Equals", 0xBB, &["=", "Plus"]),
    ("Backtick", 0xC0, &["`", "Grave"]),
    ("OpenBracket", 0xDB, &["["]),
    ("CloseBracket", 0xDD, &["]"]),
    ("Backslash", 0xDC, &["\\"]),
    ("Semicolon", 0xBA, &[";"]),
    ("Quote", 0xDE, &["'"]),
    ("Comma", 0xBC, &[","]),
    ("Period", 0xBE, &["."]),
    ("Slash", 0xBF, &["/"]),
];

/// Resolve a key name (case-insensitive, aliases allowed) to
/// (canonical name, Windows virtual-key code).
pub fn lookup_key(name: &str) -> Option<(String, u32)> {
    let n = name.trim();
    if n.is_empty() {
        return None;
    }
    let up = n.to_ascii_uppercase();
    // Single letter or digit.
    if up.len() == 1 {
        let c = up.as_bytes()[0];
        if c.is_ascii_uppercase() || c.is_ascii_digit() {
            return Some((up, c as u32));
        }
    }
    // F1..F24
    if let Some(num) = up.strip_prefix('F').and_then(|d| d.parse::<u32>().ok()) {
        if (1..=24).contains(&num) {
            return Some((format!("F{num}"), 0x70 + num - 1));
        }
    }
    // Numpad0..9 (also "Num0" is NOT numpad in egui, so only explicit names)
    if let Some(num) = up
        .strip_prefix("NUMPAD")
        .and_then(|d| d.parse::<u32>().ok())
    {
        if num <= 9 {
            return Some((format!("Numpad{num}"), 0x60 + num));
        }
    }
    for (canon, vk, aliases) in NAMED {
        if canon.eq_ignore_ascii_case(n) || aliases.iter().any(|a| a.eq_ignore_ascii_case(n)) {
            return Some((canon.to_string(), *vk));
        }
    }
    None
}

/// X11 keysym for a canonical key name.
pub fn x11_keysym(name: &str) -> Option<u32> {
    let (canon, vk) = lookup_key(name)?;
    Some(match canon.as_str() {
        c if c.len() == 1 && c.as_bytes()[0].is_ascii_uppercase() => {
            c.as_bytes()[0].to_ascii_lowercase() as u32
        }
        c if c.len() == 1 && c.as_bytes()[0].is_ascii_digit() => c.as_bytes()[0] as u32,
        c if c.starts_with('F') && c.len() > 1 && c[1..].bytes().all(|b| b.is_ascii_digit()) => {
            0xFFBE + (vk - 0x70)
        }
        c if c.starts_with("Numpad") && c.len() == 7 => 0xFFB0 + (vk - 0x60),
        "Space" => 0x20,
        "Enter" => 0xFF0D,
        "Tab" => 0xFF09,
        "Backspace" => 0xFF08,
        "Escape" => 0xFF1B,
        "Insert" => 0xFF63,
        "Delete" => 0xFFFF,
        "Home" => 0xFF50,
        "End" => 0xFF57,
        "PageUp" => 0xFF55,
        "PageDown" => 0xFF56,
        "Left" => 0xFF51,
        "Up" => 0xFF52,
        "Right" => 0xFF53,
        "Down" => 0xFF54,
        "Pause" => 0xFF13,
        "ScrollLock" => 0xFF14,
        "PrintScreen" => 0xFF61,
        "NumpadMultiply" => 0xFFAA,
        "NumpadAdd" => 0xFFAB,
        "NumpadSubtract" => 0xFFAD,
        "NumpadDecimal" => 0xFFAE,
        "NumpadDivide" => 0xFFAF,
        "Minus" => 0x2D,
        "Equals" => 0x3D,
        "Backtick" => 0x60,
        "OpenBracket" => 0x5B,
        "CloseBracket" => 0x5D,
        "Backslash" => 0x5C,
        "Semicolon" => 0x3B,
        "Quote" => 0x27,
        "Comma" => 0x2C,
        "Period" => 0x2E,
        "Slash" => 0x2F,
        _ => return None,
    })
}

/// macOS virtual key code (kVK_*) for a canonical key name.
pub fn mac_keycode(name: &str) -> Option<u32> {
    let (canon, _) = lookup_key(name)?;
    const LETTERS: [u32; 26] = [
        0x00, 0x0B, 0x08, 0x02, 0x0E, 0x03, 0x05, 0x04, 0x22, 0x26, 0x28, 0x25, 0x2E, 0x2D, 0x1F,
        0x23, 0x0C, 0x0F, 0x01, 0x11, 0x20, 0x09, 0x0D, 0x07, 0x10, 0x06,
    ];
    const DIGITS: [u32; 10] = [0x1D, 0x12, 0x13, 0x14, 0x15, 0x17, 0x16, 0x1A, 0x1C, 0x19];
    const FKEYS: [u32; 20] = [
        0x7A, 0x78, 0x63, 0x76, 0x60, 0x61, 0x62, 0x64, 0x65, 0x6D, 0x67, 0x6F, 0x69, 0x6B, 0x71,
        0x6A, 0x40, 0x4F, 0x50, 0x5A,
    ];
    const KEYPAD: [u32; 10] = [0x52, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5B, 0x5C];
    let b = canon.as_bytes();
    if b.len() == 1 && b[0].is_ascii_uppercase() {
        return Some(LETTERS[(b[0] - b'A') as usize]);
    }
    if b.len() == 1 && b[0].is_ascii_digit() {
        return Some(DIGITS[(b[0] - b'0') as usize]);
    }
    if let Some(n) = canon
        .strip_prefix('F')
        .and_then(|d| d.parse::<usize>().ok())
    {
        return FKEYS.get(n.checked_sub(1)?).copied();
    }
    if let Some(n) = canon
        .strip_prefix("Numpad")
        .and_then(|d| d.parse::<usize>().ok())
    {
        return KEYPAD.get(n).copied();
    }
    Some(match canon.as_str() {
        "Space" => 0x31,
        "Enter" => 0x24,
        "Tab" => 0x30,
        "Backspace" => 0x33,
        "Escape" => 0x35,
        "Delete" => 0x75,
        "Home" => 0x73,
        "End" => 0x77,
        "PageUp" => 0x74,
        "PageDown" => 0x79,
        "Left" => 0x7B,
        "Right" => 0x7C,
        "Down" => 0x7D,
        "Up" => 0x7E,
        "NumpadMultiply" => 0x43,
        "NumpadAdd" => 0x45,
        "NumpadSubtract" => 0x4E,
        "NumpadDecimal" => 0x41,
        "NumpadDivide" => 0x4B,
        "Minus" => 0x1B,
        "Equals" => 0x18,
        "Backtick" => 0x32,
        "OpenBracket" => 0x21,
        "CloseBracket" => 0x1E,
        "Backslash" => 0x2A,
        "Semicolon" => 0x29,
        "Quote" => 0x27,
        "Comma" => 0x2B,
        "Period" => 0x2F,
        "Slash" => 0x2C,
        _ => return None,
    })
}

impl Hotkey {
    pub fn parse(s: &str) -> Option<Hotkey> {
        let mut hk = Hotkey::default();
        for part in s.split('+').map(str::trim).filter(|p| !p.is_empty()) {
            match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" => hk.alt = true,
                "shift" => hk.shift = true,
                "win" | "super" | "meta" | "cmd" => hk.win = true,
                _ => {
                    if !hk.key.is_empty() {
                        return None; // two non-modifier keys
                    }
                    hk.key = lookup_key(part)?.0;
                }
            }
        }
        // "Ctrl++" style: trailing '+' means the Plus key.
        if hk.key.is_empty() && s.trim_end().ends_with("++") {
            hk.key = "Equals".into();
        }
        if hk.key.is_empty() {
            None
        } else {
            Some(hk)
        }
    }

    pub fn vk(&self) -> Option<u32> {
        lookup_key(&self.key).map(|k| k.1)
    }
}

impl std::fmt::Display for Hotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.ctrl {
            f.write_str("Ctrl+")?;
        }
        if self.alt {
            f.write_str("Alt+")?;
        }
        if self.shift {
            f.write_str("Shift+")?;
        }
        if self.win {
            f.write_str("Win+")?;
        }
        f.write_str(&self.key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        let h = Hotkey::parse("ctrl + shift + o").unwrap();
        assert!(h.ctrl && h.shift && !h.alt);
        assert_eq!(h.vk(), Some(0x4F));
        assert_eq!(h.to_string(), "Ctrl+Shift+O");
        assert_eq!(Hotkey::parse("Alt+F10").unwrap().vk(), Some(0x79));
        assert_eq!(Hotkey::parse("Ctrl+ArrowUp").unwrap().key, "Up");
        assert!(Hotkey::parse("Ctrl+Shift").is_none());
        assert!(Hotkey::parse("A+B").is_none());
    }

    #[test]
    fn platform_key_codes() {
        assert_eq!(x11_keysym("O"), Some(0x6F));
        assert_eq!(x11_keysym("F1"), Some(0xFFBE));
        assert_eq!(x11_keysym("F12"), Some(0xFFC9));
        assert_eq!(x11_keysym("Numpad5"), Some(0xFFB5));
        assert_eq!(mac_keycode("O"), Some(0x1F));
        assert_eq!(mac_keycode("A"), Some(0x00));
        assert_eq!(mac_keycode("F10"), Some(0x6D));
        assert_eq!(mac_keycode("0"), Some(0x1D));
    }
}
