//! Home-screen icons, drawn on paper with enough margin that a round mask doesn't clip them: a magnifying glass for
//! a search page, and a globe for a map or place search.

#[derive(Clone, Copy)]
pub enum Icon {
    Search,
    Globe,
}

impl Icon {
    pub fn named(name: &str) -> Option<Icon> {
        match name {
            "search" => Some(Icon::Search),
            "globe" => Some(Icon::Globe),
            _ => None,
        }
    }

    pub fn svg(self) -> &'static str {
        match self {
            Icon::Search => r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#fbfaf7"/><circle cx="44" cy="44" r="18.5" fill="none" stroke="#1d1c1a" stroke-width="7"/><line x1="57.5" y1="57.5" x2="74" y2="74" stroke="#9b2c1f" stroke-width="8" stroke-linecap="round"/></svg>
"##,
            Icon::Globe => r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect width="100" height="100" fill="#ffffff"/><ellipse cx="50" cy="50" rx="11" ry="30" fill="none" stroke="#000000" stroke-width="4"/><line x1="20" y1="50" x2="80" y2="50" stroke="#000000" stroke-width="4"/><circle cx="50" cy="50" r="30" fill="none" stroke="#000000" stroke-width="7"/><circle cx="68.5" cy="40.5" r="5" fill="#9b2c1f"/></svg>
"##,
        }
    }

    /// The paper color, which the page's manifest uses for its splash screen.
    pub fn paper(self) -> &'static str {
        match self {
            Icon::Search => "#fbfaf7",
            Icon::Globe => "#ffffff",
        }
    }

    /// The color at a point of the SVG's 100 by 100 drawing.
    fn color(self, px: f32, py: f32) -> [u8; 3] {
        const ACCENT: [u8; 3] = [0x9b, 0x2c, 0x1f];
        match self {
            Icon::Search => {
                let ring = ((px - 44.0).hypot(py - 44.0) - 18.5).abs() <= 3.5;
                // Distance to the handle's line from (57.5, 57.5) to (74, 74), with round ends.
                let t = (((px - 57.5) + (py - 57.5)) / 2.0 / 16.5).clamp(0.0, 1.0);
                let (hx, hy) = (57.5 + 16.5 * t, 57.5 + 16.5 * t);
                let handle = (px - hx).hypot(py - hy) <= 4.0;
                if handle { ACCENT } else if ring { [0x1d, 0x1c, 0x1a] } else { [0xfb, 0xfa, 0xf7] }
            }
            Icon::Globe => {
                let (dx, dy) = (px - 50.0, py - 50.0);
                let r = dx.hypot(dy);
                let ring = (r - 30.0).abs() <= 3.5;
                let equator = dy.abs() <= 2.0 && r <= 30.0;
                // Distance to the meridian's ellipse, to first order: how far off 1 its equation is, over its slope.
                let (ex, ey) = (dx / 11.0, dy / 30.0);
                let f = ex.hypot(ey);
                let slope = (dx / 121.0).hypot(dy / 900.0) / f.max(1e-6);
                let meridian = (f - 1.0).abs() / slope.max(1e-6) <= 2.0;
                let place = (px - 68.5).hypot(py - 40.5) <= 5.0;
                if place { ACCENT } else if ring || equator || meridian { [0, 0, 0] } else { [0xff, 0xff, 0xff] }
            }
        }
    }

    /// The drawing as a PNG `size` pixels square.
    pub fn png(self, size: usize) -> Vec<u8> {
        let unit = size as f32 / 100.0;
        let mut raw = Vec::with_capacity(size * (size * 3 + 1));
        for y in 0..size {
            raw.push(0);
            for x in 0..size {
                raw.extend(self.color((x as f32 + 0.5) / unit, (y as f32 + 0.5) / unit));
            }
        }
        let mut header = Vec::new();
        header.extend((size as u32).to_be_bytes());
        header.extend((size as u32).to_be_bytes());
        header.extend([8, 2, 0, 0, 0]); // 8-bit RGB
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        chunk(&mut out, b"IHDR", &header);
        chunk(&mut out, b"IDAT", &zlib(&raw, size * 3 + 1));
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend(kind);
    out.extend(data);
    let crc = crc32(&out[start..]);
    out.extend(crc.to_be_bytes());
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &b in bytes {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xedb8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Deflate with the fixed codes, copying runs from one byte back or one row back: enough for flat colors.
fn zlib(data: &[u8], row: usize) -> Vec<u8> {
    let mut bits = Bits::default();
    bits.put(1, 1); // the last block
    bits.put(1, 2); // fixed codes
    let mut i = 0;
    while i < data.len() {
        let run = |distance: usize| -> usize {
            if i < distance {
                return 0;
            }
            let mut n = 0;
            while n < 258 && i + n < data.len() && data[i + n] == data[i + n - distance] {
                n += 1;
            }
            n
        };
        let (back, same) = [(row, run(row)), (3, run(3)), (1, run(1))].into_iter().max_by_key(|&(_, n)| n).unwrap();
        if same >= 3 {
            bits.length(same);
            bits.distance(back);
            i += same;
        } else {
            bits.literal(data[i] as u32);
            i += 1;
        }
    }
    bits.literal(256);
    let mut out = vec![0x78, 0x01];
    out.extend(bits.finish());
    let (mut a, mut b) = (1u32, 0u32);
    for &byte in data {
        a = (a + byte as u32) % 65521;
        b = (b + a) % 65521;
    }
    out.extend(((b << 16) | a).to_be_bytes());
    out
}

#[derive(Default)]
struct Bits {
    out: Vec<u8>,
    acc: u32,
    n: u32,
}

const LENGTHS: [u16; 29] = [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LENGTH_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
const DISTANCES: [u16; 30] = [1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577];
const DISTANCE_EXTRA: [u8; 30] = [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];

impl Bits {
    /// `count` bits of `value`, lowest first.
    fn put(&mut self, value: u32, count: u32) {
        for k in 0..count {
            self.acc |= ((value >> k) & 1) << self.n;
            self.n += 1;
            if self.n == 8 {
                self.out.push(self.acc as u8);
                self.acc = 0;
                self.n = 0;
            }
        }
    }

    /// A Huffman code, highest bit first.
    fn code(&mut self, code: u32, len: u32) {
        for k in (0..len).rev() {
            self.put((code >> k) & 1, 1);
        }
    }

    fn literal(&mut self, v: u32) {
        match v {
            0..=143 => self.code(0x30 + v, 8),
            144..=255 => self.code(0x190 + v - 144, 9),
            256..=279 => self.code(v - 256, 7),
            _ => self.code(0xc0 + v - 280, 8),
        }
    }

    fn length(&mut self, n: usize) {
        let k = LENGTHS.iter().rposition(|&l| l as usize <= n).unwrap();
        self.literal(257 + k as u32);
        self.put((n - LENGTHS[k] as usize) as u32, LENGTH_EXTRA[k] as u32);
    }

    fn distance(&mut self, d: usize) {
        let k = DISTANCES.iter().rposition(|&l| l as usize <= d).unwrap();
        self.code(k as u32, 5);
        self.put((d - DISTANCES[k] as usize) as u32, DISTANCE_EXTRA[k] as u32);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}
