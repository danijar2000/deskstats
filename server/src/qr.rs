//! A small QR code encoder, for showing the link to the phone app in a terminal: bytes, error correction level L,
//! versions 1 to 6 (up to 134 bytes), which is plenty for a URL. After ISO/IEC 18004.

/// Data codewords in all, and error correction codewords per block, for versions 1 to 6 at level L.
/// Version 6 has two blocks; the others have one.
const DATA: [usize; 6] = [19, 34, 55, 80, 108, 136];
const ERROR: [usize; 6] = [7, 10, 15, 20, 26, 18];

pub struct Qr {
    pub size: usize,
    dark: Vec<bool>,
}

impl Qr {
    pub fn dark(&self, x: usize, y: usize) -> bool {
        self.dark[y * self.size + x]
    }

    /// The smallest code that holds [text]; None when it is longer than 134 bytes.
    pub fn encode(text: &str) -> Option<Qr> {
        let bytes = text.as_bytes();
        // Four bits of mode and eight of length come before the data.
        let version = (1..=6).find(|v| bytes.len() + 2 <= DATA[v - 1])?;
        let codewords = interleaved(&data_codewords(bytes, DATA[version - 1]), version);
        let mut best: Option<(u32, Qr)> = None;
        for mask in 0..8 {
            let qr = draw(version, &codewords, mask);
            let penalty = qr.penalty();
            if best.as_ref().is_none_or(|(least, _)| penalty < *least) {
                best = Some((penalty, qr));
            }
        }
        best.map(|(_, qr)| qr)
    }

    /// The code for a terminal, two rows of modules per line of text, black on white with a quiet border.
    pub fn terminal(&self) -> String {
        const BORDER: isize = 3;
        let size = self.size as isize;
        let at = |x: isize, y: isize| x >= 0 && y >= 0 && x < size && y < size && self.dark(x as usize, y as usize);
        let mut out = String::new();
        let mut y = -BORDER;
        while y < size + BORDER {
            out.push_str("\x1b[30;47m");
            for x in -BORDER..size + BORDER {
                out.push(match (at(x, y), at(x, y + 1)) {
                    (true, true) => '█',
                    (true, false) => '▀',
                    (false, true) => '▄',
                    (false, false) => ' ',
                });
            }
            out.push_str("\x1b[0m\n");
            y += 2;
        }
        out
    }

    /// The code as lines of 0 and 1, for a program that draws it itself.
    pub fn bits(&self) -> String {
        (0..self.size).map(|y| (0..self.size).map(|x| if self.dark(x, y) { '1' } else { '0' }).collect::<String>() + "\n").collect()
    }

    /// How hard the code is to read, by the four rules of the standard; the mask with the least is used.
    fn penalty(&self) -> u32 {
        let n = self.size;
        let mut total = 0u32;
        for swap in [false, true] {
            let at = |a: usize, b: usize| if swap { self.dark(a, b) } else { self.dark(b, a) };
            for a in 0..n {
                // Runs of five or more of one colour.
                let mut run = 1;
                for b in 1..=n {
                    if b < n && at(a, b) == at(a, b - 1) {
                        run += 1;
                    } else {
                        if run >= 5 {
                            total += 3 + (run - 5);
                        }
                        run = 1;
                    }
                }
                // What looks like the pattern in the corners.
                for b in 0..n.saturating_sub(10) {
                    let line: Vec<bool> = (0..11).map(|i| at(a, b + i)).collect();
                    const FINDER: [bool; 11] = [true, false, true, true, true, false, true, false, false, false, false];
                    if line.iter().eq(FINDER.iter()) || line.iter().rev().eq(FINDER.iter()) {
                        total += 40;
                    }
                }
            }
        }
        // Blocks of two by two.
        for y in 0..n - 1 {
            for x in 0..n - 1 {
                let c = self.dark(x, y);
                if c == self.dark(x + 1, y) && c == self.dark(x, y + 1) && c == self.dark(x + 1, y + 1) {
                    total += 3;
                }
            }
        }
        // Far from half dark.
        let dark = self.dark.iter().filter(|d| **d).count() as i64;
        let all = (n * n) as i64;
        total + ((dark * 20 - all * 10).abs() / all) as u32 * 10
    }
}

/// Mode, length, the bytes, the terminator and the padding, filled up to [capacity] codewords.
fn data_codewords(bytes: &[u8], capacity: usize) -> Vec<u8> {
    let mut bits: Vec<bool> = Vec::with_capacity(capacity * 8);
    let mut push = |value: u32, count: u32| (0..count).rev().for_each(|i| bits.push(value >> i & 1 == 1));
    push(0b0100, 4);
    push(bytes.len() as u32, 8);
    bytes.iter().for_each(|b| push(*b as u32, 8));
    let end = (bits.len() + 4).min(capacity * 8);
    bits.resize(end, false);
    bits.resize(bits.len().div_ceil(8) * 8, false);
    let mut out: Vec<u8> = bits.chunks(8).map(|byte| byte.iter().fold(0u8, |acc, bit| acc << 1 | *bit as u8)).collect();
    for pad in [0xEC, 0x11].iter().cycle() {
        if out.len() >= capacity {
            break;
        }
        out.push(*pad);
    }
    out
}

/// Multiplication in the field of the Reed–Solomon code (GF(256) modulo x⁸+x⁴+x³+x²+1).
fn multiply(a: u8, b: u8) -> u8 {
    let mut z = 0u8;
    for i in (0..8).rev() {
        z = (z << 1) ^ ((z >> 7) * 0x1D);
        z ^= ((b >> i) & 1) * a;
    }
    z
}

/// The error correction codewords of one block.
fn error_codewords(data: &[u8], count: usize) -> Vec<u8> {
    // The generator polynomial (x - 2⁰)(x - 2¹)…, highest power left out.
    let mut generator = vec![0u8; count];
    generator[count - 1] = 1;
    let mut root = 1u8;
    for _ in 0..count {
        for j in 0..count {
            generator[j] = multiply(generator[j], root);
            if j + 1 < count {
                generator[j] ^= generator[j + 1];
            }
        }
        root = multiply(root, 2);
    }
    let mut remainder = vec![0u8; count];
    for byte in data {
        let factor = byte ^ remainder.remove(0);
        remainder.push(0);
        for (r, g) in remainder.iter_mut().zip(&generator) {
            *r ^= multiply(*g, factor);
        }
    }
    remainder
}

/// The data and error correction codewords in the order they are drawn: block by block for one block, and
/// taking turns between the blocks when there are two.
fn interleaved(data: &[u8], version: usize) -> Vec<u8> {
    let blocks: Vec<&[u8]> = if version == 6 { data.chunks(data.len() / 2).collect() } else { vec![data] };
    let errors: Vec<Vec<u8>> = blocks.iter().map(|block| error_codewords(block, ERROR[version - 1])).collect();
    let mut out = Vec::new();
    for i in 0..blocks[0].len() {
        blocks.iter().for_each(|block| out.push(block[i]));
    }
    for i in 0..ERROR[version - 1] {
        errors.iter().for_each(|block| out.push(block[i]));
    }
    out
}

fn draw(version: usize, codewords: &[u8], mask: u32) -> Qr {
    let size = 17 + 4 * version;
    let mut dark = vec![false; size * size];
    let mut fixed = vec![false; size * size];
    let mut set = |x: usize, y: usize, on: bool, dark: &mut Vec<bool>| {
        dark[y * size + x] = on;
        fixed[y * size + x] = true;
    };
    // The dotted lines between the corners.
    for i in 0..size {
        set(6, i, i % 2 == 0, &mut dark);
        set(i, 6, i % 2 == 0, &mut dark);
    }
    // The three corners, each with a light border around it.
    for (cx, cy) in [(3isize, 3isize), (size as isize - 4, 3), (3, size as isize - 4)] {
        for dy in -4isize..=4 {
            for dx in -4isize..=4 {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as usize) < size && (y as usize) < size {
                    let ring = dx.abs().max(dy.abs());
                    set(x as usize, y as usize, ring != 2 && ring != 4, &mut dark);
                }
            }
        }
    }
    // The small square near the fourth corner, from version 2 on.
    if version >= 2 {
        let c = size as isize - 7;
        for dy in -2isize..=2 {
            for dx in -2isize..=2 {
                set((c + dx) as usize, (c + dy) as usize, dx.abs().max(dy.abs()) != 1, &mut dark);
            }
        }
    }
    // Which mask is used and the level, protected by its own code, twice.
    let data = 1 << 3 | mask;
    let mut remainder = data;
    for _ in 0..10 {
        remainder = (remainder << 1) ^ ((remainder >> 9) * 0x537);
    }
    let format = (data << 10 | remainder) ^ 0x5412;
    let bit = |i: usize| format >> i & 1 == 1;
    for i in 0..6 {
        set(8, i, bit(i), &mut dark);
    }
    set(8, 7, bit(6), &mut dark);
    set(8, 8, bit(7), &mut dark);
    set(7, 8, bit(8), &mut dark);
    for i in 9..15 {
        set(14 - i, 8, bit(i), &mut dark);
    }
    for i in 0..8 {
        set(size - 1 - i, 8, bit(i), &mut dark);
    }
    for i in 8..15 {
        set(8, size - 15 + i, bit(i), &mut dark);
    }
    set(8, size - 8, true, &mut dark);

    // The codewords, in a zigzag of two columns from the bottom right, with the mask over them.
    let masked = |x: usize, y: usize| match mask {
        0 => (x + y) % 2 == 0,
        1 => y % 2 == 0,
        2 => x % 3 == 0,
        3 => (x + y) % 3 == 0,
        4 => (x / 3 + y / 2) % 2 == 0,
        5 => x * y % 2 + x * y % 3 == 0,
        6 => (x * y % 2 + x * y % 3) % 2 == 0,
        _ => ((x + y) % 2 + x * y % 3) % 2 == 0,
    };
    let mut i = 0;
    let mut right = size as isize - 1;
    while right >= 1 {
        if right == 6 {
            right = 5;
        }
        for vertical in 0..size {
            for j in 0..2 {
                let x = (right - j) as usize;
                let upward = (right + 1) & 2 == 0;
                let y = if upward { size - 1 - vertical } else { vertical };
                if fixed[y * size + x] {
                    continue;
                }
                // The few modules left after the last codeword stay light, before the mask.
                let on = i < codewords.len() * 8 && codewords[i / 8] >> (7 - i % 8) & 1 == 1;
                i += 1;
                dark[y * size + x] = on ^ masked(x, y);
            }
        }
        right -= 2;
    }
    Qr { size, dark }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_correction_matches_the_standard() {
        // The example of the standard: "01234567" in version 1-M has these ten error correction codewords.
        let data = [0x10, 0x20, 0x0C, 0x56, 0x61, 0x80, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11, 0xEC, 0x11];
        assert_eq!(error_codewords(&data, 10), [0xA5, 0x24, 0xD4, 0xC1, 0xED, 0x36, 0xC7, 0x87, 0x2C, 0x55]);
        assert_eq!(multiply(2, 0x80), 0x1D);
    }

    #[test]
    fn data_is_framed_and_padded() {
        assert_eq!(data_codewords(b"A", 5), [0x40, 0x14, 0x10, 0xEC, 0x11]);
        assert_eq!(data_codewords(&[0xFF; 17], 19).len(), 19);
        assert_eq!(data_codewords(&[0xFF; 17], 19)[18], 0xF0);
    }

    #[test]
    fn sizes_and_fixed_patterns() {
        let small = Qr::encode("hi").unwrap();
        assert_eq!(small.size, 21);
        let link = Qr::encode("https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk").unwrap();
        assert_eq!(link.size, 37);
        for qr in [&small, &link] {
            let n = qr.size;
            // The corners: dark frame, light ring, dark middle.
            for (x, y) in [(0, 0), (n - 7, 0), (0, n - 7)] {
                assert!(qr.dark(x, y) && qr.dark(x + 6, y + 6) && !qr.dark(x + 1, y + 1) && qr.dark(x + 3, y + 3));
            }
            assert!(qr.dark(8, n - 8));
            assert!(qr.dark(8, 6) && !qr.dark(9, 6) && qr.dark(10, 6));
        }
        assert!(Qr::encode(&"x".repeat(134)).is_some_and(|qr| qr.size == 41));
        assert!(Qr::encode(&"x".repeat(135)).is_none());
        assert_eq!(small.bits().lines().count(), 21);
        assert!(small.terminal().lines().count() >= 13);
    }

    /// The whole code for the link to the app, compared with one made by the python-qrcode library.
    #[test]
    fn the_link_matches_a_reference_encoder() {
        let qr = Qr::encode("https://github.com/danijar2000/deskstats/releases/latest/download/deskstats.apk").unwrap();
        assert_eq!(crate::crypto::hex(&crate::crypto::sha256(qr.bits().as_bytes())), "a3dae8481e81e18fa48a265ac2bfada8ac5e6abe94db3bb1c26ebf48b5cddd60");
    }
}
