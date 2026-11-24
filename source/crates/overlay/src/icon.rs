//! App icon export: PNG and ICO files from the same procedurally drawn
//! icon the tray uses. Installers call `universal-overlay --export-icon
//! <file> [size]`, so no image files or converters are needed at build time.
//! The PNG encoder carries its own tiny deflate (LZ77 + fixed Huffman), so
//! icons are a few KB and no compression library is linked.

use crate::core::icon_pixels;

fn crc32(data: &[u8]) -> u32 {
    let mut c = !0u32;
    for &b in data {
        c ^= b as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
        }
    }
    !c
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, ty: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(ty);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

/// RGBA PNG of the app icon, `size` × `size` pixels.
pub fn png(size: i32) -> Vec<u8> {
    let size = size.clamp(8, 1024);
    encode_png(size, size, &icon_pixels(size))
}

/// PNG from straight-alpha 0xAARRGGBB pixels.
pub fn encode_png(w: i32, h: i32, px: &[u32]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(px.len() * 4);
    for &p in px {
        rgba.extend_from_slice(&[(p >> 16) as u8, (p >> 8) as u8, p as u8, (p >> 24) as u8]);
    }
    let raw = filter_rows(&rgba, w.max(1) as usize);
    let mut z = vec![0x78, 0x01];
    z.extend_from_slice(&deflate(&raw));
    z.extend_from_slice(&adler32(&raw).to_be_bytes());

    let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Windows .ico with PNG-compressed images (supported since Vista).
pub fn ico() -> Vec<u8> {
    let sizes = [16, 20, 24, 32, 40, 48, 64, 128, 256];
    let images: Vec<Vec<u8>> = sizes.iter().map(|&s| png(s)).collect();
    let mut out = Vec::new();
    out.extend_from_slice(&[0, 0, 1, 0]);
    out.extend_from_slice(&(sizes.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * sizes.len() as u32;
    for (&s, img) in sizes.iter().zip(&images) {
        let dim = if s >= 256 { 0 } else { s as u8 };
        out.extend_from_slice(&[dim, dim, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes()); // planes
        out.extend_from_slice(&32u16.to_le_bytes()); // bpp
        out.extend_from_slice(&(img.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += img.len() as u32;
    }
    for img in images {
        out.extend_from_slice(&img);
    }
    out
}

/// Handle `--export-icon <file> [size]`. Returns true if the flag was given.
pub fn export_from_args() -> bool {
    let args: Vec<String> = std::env::args().collect();
    let Some(i) = args.iter().position(|a| a == "--export-icon") else {
        return false;
    };
    let Some(path) = args.get(i + 1) else {
        eprintln!("--export-icon <file.png|file.ico> [size]");
        std::process::exit(2);
    };
    let size: i32 = args.get(i + 2).and_then(|s| s.parse().ok()).unwrap_or(256);
    let data = if path.to_lowercase().ends_with(".ico") {
        ico()
    } else {
        png(size)
    };
    if let Err(e) = std::fs::write(path, data) {
        eprintln!("{path}: {e}");
        std::process::exit(1);
    }
    true
}

// ---------------------------------------------------------------- deflate

/// LSB-first bit writer for deflate.
struct Bits {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Bits {
    fn put(&mut self, v: u32, bits: u32) {
        self.acc |= (v as u64) << self.n;
        self.n += bits;
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    /// Huffman codes are stored MSB-first.
    fn code(&mut self, code: u32, len: u32) {
        let mut r = 0;
        for i in 0..len {
            r |= ((code >> i) & 1) << (len - 1 - i);
        }
        self.put(r, len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

/// Literal/length symbol with the fixed Huffman table (RFC 1951, 3.2.6).
fn fixed_sym(b: &mut Bits, sym: u32) {
    match sym {
        0..=143 => b.code(0x30 + sym, 8),
        144..=255 => b.code(0x190 + sym - 144, 9),
        256..=279 => b.code(sym - 256, 7),
        _ => b.code(0xC0 + sym - 280, 8),
    }
}

/// Raw deflate stream: LZ77 (hash chains, 32 KiB window) + fixed Huffman
/// codes. Small and fast; flat-colour images shrink 20–50×.
fn deflate(data: &[u8]) -> Vec<u8> {
    const WIN: usize = 32 * 1024;
    const HASH: usize = 1 << 15;
    const CHAIN: usize = 32;
    let mut b = Bits {
        out: Vec::with_capacity(data.len() / 4 + 16),
        acc: 0,
        n: 0,
    };
    b.put(1, 1); // final block
    b.put(1, 2); // fixed Huffman
    let mut head = vec![usize::MAX; HASH];
    let mut prev = vec![usize::MAX; data.len()];
    let hash = |i: usize| {
        ((data[i] as usize) << 10 ^ (data[i + 1] as usize) << 5 ^ data[i + 2] as usize) & (HASH - 1)
    };
    let insert = |i: usize, head: &mut Vec<usize>, prev: &mut Vec<usize>| {
        if i + 2 < data.len() {
            let h = hash(i);
            prev[i] = head[h];
            head[h] = i;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0usize, 0usize);
        if i + 2 < data.len() {
            let mut cand = head[hash(i)];
            let mut steps = 0;
            while cand != usize::MAX && i - cand <= WIN && steps < CHAIN {
                let max = (data.len() - i).min(258);
                let mut l = 0;
                while l < max && data[cand + l] == data[i + l] {
                    l += 1;
                }
                if l > best_len {
                    best_len = l;
                    best_dist = i - cand;
                    if l == max {
                        break;
                    }
                }
                cand = prev[cand];
                steps += 1;
            }
        }
        if best_len >= 3 {
            let li = LEN_BASE
                .iter()
                .rposition(|&x| x as usize <= best_len)
                .unwrap();
            fixed_sym(&mut b, 257 + li as u32);
            b.put(
                (best_len - LEN_BASE[li] as usize) as u32,
                LEN_EXTRA[li] as u32,
            );
            let di = DIST_BASE
                .iter()
                .rposition(|&x| x as usize <= best_dist)
                .unwrap();
            b.code(di as u32, 5);
            b.put(
                (best_dist - DIST_BASE[di] as usize) as u32,
                DIST_EXTRA[di] as u32,
            );
            for k in i..i + best_len {
                insert(k, &mut head, &mut prev);
            }
            i += best_len;
        } else {
            fixed_sym(&mut b, data[i] as u32);
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    fixed_sym(&mut b, 256); // end of block
    b.finish()
}

/// PNG scanlines with the cheapest of the None / Sub / Up filters per row.
fn filter_rows(rgba: &[u8], w: usize) -> Vec<u8> {
    let stride = w * 4;
    let mut out = Vec::with_capacity(rgba.len() + rgba.len() / stride.max(1));
    let mut cand = [vec![0u8; stride], vec![0u8; stride], vec![0u8; stride]];
    for (y, row) in rgba.chunks(stride).enumerate() {
        let up = if y > 0 {
            &rgba[(y - 1) * stride..y * stride]
        } else {
            &[][..]
        };
        for x in 0..row.len() {
            let left = if x >= 4 { row[x - 4] } else { 0 };
            let above = if y > 0 { up[x] } else { 0 };
            cand[0][x] = row[x];
            cand[1][x] = row[x].wrapping_sub(left);
            cand[2][x] = row[x].wrapping_sub(above);
        }
        let cost = |c: &Vec<u8>| {
            c[..row.len()]
                .iter()
                .map(|&v| (v as i8).unsigned_abs() as u32)
                .sum::<u32>()
        };
        let best = (0..3).min_by_key(|&k| cost(&cand[k])).unwrap();
        out.push(best as u8);
        out.extend_from_slice(&cand[best][..row.len()]);
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc_and_adler_known_values() {
        assert_eq!(super::crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(super::adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn deflate_shrinks_flat_data() {
        let data = vec![7u8; 100_000];
        let z = super::deflate(&data);
        assert!(z.len() < 1000, "{}", z.len());
    }

    #[test]
    fn png_has_signature_and_ico_header() {
        assert!(super::png(32).starts_with(b"\x89PNG"));
        let ico = super::ico();
        assert_eq!(&ico[..4], &[0, 0, 1, 0]);
    }
}
