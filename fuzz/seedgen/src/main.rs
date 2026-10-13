//! Generates seed corpora for the differential fuzzers.
//!
//! usage: woff2-seedgen <out-dir> [font files or directories...]
//!
//! Writes `<out-dir>/{encode,decode,decode_structured,common}`. Fonts are
//! synthesized from scratch (so the corpus is free of third-party font
//! data); any fonts given on the command line are additionally minified and
//! added. With `--regression`, the given fonts are also written unmodified
//! (with encode parameters) to `<out-dir>/regression_{encode,decode}`.

use std::fs;
use std::os::raw::c_int;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn below(&mut self, n: u64) -> u64 {
        if n == 0 {
            0
        } else {
            self.next() % n
        }
    }
    fn range(&mut self, lo: i64, hi: i64) -> i64 {
        lo + self.below((hi - lo + 1) as u64) as i64
    }
    fn chance(&mut self, p: u64) -> bool {
        self.below(100) < p
    }
    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| self.next() as u8).collect()
    }
}

fn tag(t: &[u8; 4]) -> u32 {
    u32::from_be_bytes(*t)
}

fn be16(v: &mut Vec<u8>, x: i64) {
    v.extend_from_slice(&(x as u16).to_be_bytes());
}
fn be32(v: &mut Vec<u8>, x: u32) {
    v.extend_from_slice(&x.to_be_bytes());
}
fn rd16(d: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_be_bytes(d.get(o..o + 2)?.try_into().ok()?))
}
fn rd32(d: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_be_bytes(d.get(o..o + 4)?.try_into().ok()?))
}

type Tables = Vec<(u32, Vec<u8>)>;

fn pad4(v: &mut Vec<u8>) {
    while v.len() % 4 != 0 {
        v.push(0);
    }
}

fn offset_table(out: &mut Vec<u8>, flavor: u32, n: usize) {
    be32(out, flavor);
    be16(out, n as i64);
    let mut p = 0;
    while (1usize << (p + 1)) <= n {
        p += 1;
    }
    be16(out, (16usize << p) as i64);
    be16(out, p as i64);
    be16(out, (n * 16 - (16usize << p)) as i64);
}

fn write_sfnt(flavor: u32, tables: &Tables) -> Vec<u8> {
    let mut sorted = tables.clone();
    sorted.sort_by_key(|t| t.0);
    let mut out = Vec::new();
    offset_table(&mut out, flavor, sorted.len());
    let mut offset = 12 + 16 * sorted.len();
    for (t, d) in &sorted {
        be32(&mut out, *t);
        be32(&mut out, woff2::common::compute_ulong_sum(d));
        be32(&mut out, offset as u32);
        be32(&mut out, d.len() as u32);
        offset += (d.len() + 3) & !3;
    }
    for (_, d) in &sorted {
        out.extend_from_slice(d);
        pad4(&mut out);
    }
    out
}

/// Writes a collection; fonts reference table blobs by index, so tables can
/// be shared between fonts.
fn write_ttc(version: u32, fonts: &[(u32, Vec<(u32, usize)>)], blobs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, tag(b"ttcf"));
    be32(&mut out, version);
    be32(&mut out, fonts.len() as u32);
    let header = 12 + 4 * fonts.len() + if version == 0x0002_0000 { 12 } else { 0 };
    let mut font_offsets = Vec::new();
    let mut o = header;
    for (_, tables) in fonts {
        font_offsets.push(o);
        o += 12 + 16 * tables.len();
    }
    let mut blob_offsets = Vec::new();
    for b in blobs {
        blob_offsets.push(o);
        o += (b.len() + 3) & !3;
    }
    for fo in &font_offsets {
        be32(&mut out, *fo as u32);
    }
    if version == 0x0002_0000 {
        out.extend_from_slice(&[0; 12]);
    }
    for (flavor, tables) in fonts {
        let mut sorted = tables.clone();
        sorted.sort_by_key(|t| t.0);
        offset_table(&mut out, *flavor, sorted.len());
        for (t, bi) in &sorted {
            be32(&mut out, *t);
            be32(&mut out, woff2::common::compute_ulong_sum(&blobs[*bi]));
            be32(&mut out, blob_offsets[*bi] as u32);
            be32(&mut out, blobs[*bi].len() as u32);
        }
    }
    for b in blobs {
        out.extend_from_slice(b);
        pad4(&mut out);
    }
    out
}

fn parse_sfnt_at(data: &[u8], at: usize) -> Option<(u32, Tables)> {
    let flavor = rd32(data, at)?;
    let n = rd16(data, at + 4)? as usize;
    let mut tables = Vec::new();
    for i in 0..n {
        let e = at + 12 + 16 * i;
        let t = rd32(data, e)?;
        let off = rd32(data, e + 8)? as usize;
        let len = rd32(data, e + 12)? as usize;
        tables.push((t, data.get(off..off.checked_add(len)?)?.to_vec()));
    }
    Some((flavor, tables))
}

fn parse_fonts(data: &[u8]) -> Vec<(u32, Tables)> {
    if rd32(data, 0) == Some(tag(b"ttcf")) {
        let n = rd32(data, 8).unwrap_or(0).min(64) as usize;
        (0..n)
            .filter_map(|i| parse_sfnt_at(data, rd32(data, 12 + 4 * i)? as usize))
            .collect()
    } else {
        parse_sfnt_at(data, 0).into_iter().collect()
    }
}

fn get<'a>(tables: &'a Tables, t: &[u8; 4]) -> Option<&'a Vec<u8>> {
    tables.iter().find(|x| x.0 == tag(t)).map(|x| &x.1)
}

// ---------------------------------------------------------------------------
// Minifying real fonts
// ---------------------------------------------------------------------------

/// Keeps the first `n` glyphs (TrueType outlines) and only small tables.
fn minify(tables: &Tables, n: usize, keep_all_small: bool) -> Option<Tables> {
    let head = get(tables, b"head")?;
    if head.len() < 54 {
        return None;
    }
    let mut out: Tables = Vec::new();
    let small = |t: &[u8; 4]| get(tables, t).filter(|d| d.len() <= 2048).cloned();
    if let (Some(loca), Some(glyf)) = (get(tables, b"loca"), get(tables, b"glyf")) {
        let long = head[51] != 0;
        let total = if long { loca.len() / 4 } else { loca.len() / 2 }.saturating_sub(1);
        let n = n.min(total);
        let off = |i: usize| -> usize {
            if long {
                rd32(loca, 4 * i).unwrap_or(0) as usize
            } else {
                2 * rd16(loca, 2 * i).unwrap_or(0) as usize
            }
        };
        let mut new_glyf = Vec::new();
        let mut new_loca = Vec::new();
        for i in 0..n {
            let (a, b) = (off(i), off(i + 1));
            if long {
                be32(&mut new_loca, new_glyf.len() as u32);
            } else {
                be16(&mut new_loca, (new_glyf.len() / 2) as i64);
            }
            if a <= b && b <= glyf.len() {
                new_glyf.extend_from_slice(&glyf[a..b]);
                if new_glyf.len() % 2 != 0 {
                    new_glyf.push(0);
                }
            }
        }
        if long {
            be32(&mut new_loca, new_glyf.len() as u32);
        } else {
            be16(&mut new_loca, (new_glyf.len() / 2) as i64);
        }
        out.push((tag(b"glyf"), new_glyf));
        out.push((tag(b"loca"), new_loca));
        if let (Some(hhea), Some(hmtx)) = (get(tables, b"hhea"), get(tables, b"hmtx")) {
            let mut hhea = hhea.clone();
            if hhea.len() >= 36 {
                let nh = (rd16(&hhea, 34).unwrap_or(1) as usize).clamp(1, n.max(1));
                hhea[34..36].copy_from_slice(&(nh as u16).to_be_bytes());
                let mut h = hmtx.get(..4 * nh).map(|x| x.to_vec()).unwrap_or_default();
                for i in nh..n {
                    let lsb = hmtx.get(4 * nh + 2 * (i - nh)..4 * nh + 2 * (i - nh) + 2);
                    h.extend_from_slice(lsb.unwrap_or(&[0, 0]));
                }
                out.push((tag(b"hhea"), hhea));
                out.push((tag(b"hmtx"), h));
            }
        }
        if let Some(maxp) = get(tables, b"maxp") {
            let mut maxp = maxp.clone();
            if maxp.len() >= 6 {
                maxp[4..6].copy_from_slice(&(n as u16).to_be_bytes());
            }
            out.push((tag(b"maxp"), maxp));
        }
    } else {
        for t in [b"CFF ", b"hhea", b"hmtx", b"maxp"] {
            out.extend(small(t).map(|d| (tag(t), d)));
        }
    }
    out.push((tag(b"head"), head.clone()));
    for t in [b"cvt ", b"fpgm", b"prep", b"OS/2", b"gasp", b"DSIG"] {
        out.extend(small(t).map(|d| (tag(t), d)));
    }
    if let Some(post) = get(tables, b"post") {
        let mut p = post.get(..32).map(|x| x.to_vec()).unwrap_or_default();
        if p.len() == 32 {
            p[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
            out.push((tag(b"post"), p));
        }
    }
    if keep_all_small {
        for (t, d) in tables {
            if d.len() <= 512 && !out.iter().any(|x| x.0 == *t) {
                out.push((*t, d.clone()));
            }
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Synthetic fonts
// ---------------------------------------------------------------------------

fn synth_coord_delta(rng: &mut Rng) -> i64 {
    match rng.below(6) {
        0 => 0,
        1 => rng.range(-16, 16),
        2 => rng.range(-255, 255),
        3 => rng.range(-1279, 1279),
        4 => rng.range(-4095, 4095),
        _ => rng.range(-32768, 32767),
    }
}

fn synth_simple_glyph(rng: &mut Rng) -> Vec<u8> {
    let nc = rng.range(1, 4) as usize;
    let mut counts = Vec::new();
    for i in 0..nc {
        counts.push(rng.range(if i == 0 { 1 } else { 0 }, 24) as usize);
    }
    let total: usize = counts.iter().sum();
    let mut pts = Vec::new();
    let (mut x, mut y) = (0i64, 0i64);
    let (mut xmin, mut ymin, mut xmax, mut ymax) = (i64::MAX, i64::MAX, i64::MIN, i64::MIN);
    for i in 0..total {
        let (dx, dy) = (synth_coord_delta(rng), synth_coord_delta(rng));
        x = (x + dx).clamp(-32768, 32767);
        y = (y + dy).clamp(-32768, 32767);
        let on = rng.chance(60);
        let overlap = i == 0 && rng.chance(20);
        pts.push((x, y, on, overlap));
        xmin = xmin.min(x);
        ymin = ymin.min(y);
        xmax = xmax.max(x);
        ymax = ymax.max(y);
    }
    let mut g = Vec::new();
    be16(&mut g, nc as i64);
    if rng.chance(85) {
        for v in [xmin, ymin, xmax, ymax] {
            be16(&mut g, v);
        }
    } else {
        for _ in 0..4 {
            be16(&mut g, rng.range(-2000, 2000));
        }
    }
    let mut end = -1i64;
    for c in &counts {
        end += *c as i64;
        be16(&mut g, end);
    }
    let ins = if rng.chance(30) {
        rng.range(0, 12) as usize
    } else {
        0
    };
    be16(&mut g, ins as i64);
    g.extend(rng.bytes(ins));
    // Flags: either optimal-ish short encoding or always-long (normalization
    // has something to do), with or without repeats.
    let long_coords = rng.chance(30);
    let use_repeat = rng.chance(70);
    let mut flags = Vec::new();
    let mut xs = Vec::new();
    let mut ys = Vec::new();
    let (mut lx, mut ly) = (0i64, 0i64);
    for &(px, py, on, overlap) in &pts {
        let (dx, dy) = (px - lx, py - ly);
        lx = px;
        ly = py;
        let mut f: u8 = if on { 1 } else { 0 };
        if overlap {
            f |= 0x40;
        }
        if !long_coords && dx == 0 {
            f |= 0x10;
        } else if !long_coords && dx.abs() < 256 {
            f |= 0x02 | if dx > 0 { 0x10 } else { 0 };
            xs.push(dx.unsigned_abs() as u8);
        } else {
            xs.extend_from_slice(&(dx as i16).to_be_bytes());
        }
        if !long_coords && dy == 0 {
            f |= 0x20;
        } else if !long_coords && dy.abs() < 256 {
            f |= 0x04 | if dy > 0 { 0x20 } else { 0 };
            ys.push(dy.unsigned_abs() as u8);
        } else {
            ys.extend_from_slice(&(dy as i16).to_be_bytes());
        }
        flags.push(f);
    }
    let mut i = 0;
    while i < flags.len() {
        let mut run = 1;
        while use_repeat && i + run < flags.len() && flags[i + run] == flags[i] && run < 256 {
            run += 1;
        }
        if run > 1 {
            g.push(flags[i] | 0x08);
            g.push((run - 1) as u8);
        } else {
            g.push(flags[i]);
        }
        i += run;
    }
    g.extend(xs);
    g.extend(ys);
    if rng.chance(20) {
        g.extend({
            let n = rng.below(5) as usize;
            rng.bytes(n)
        }); // trailing padding/garbage
    }
    g
}

fn synth_composite_glyph(rng: &mut Rng, num_glyphs: usize) -> Vec<u8> {
    let mut g = Vec::new();
    be16(&mut g, -1);
    for _ in 0..4 {
        be16(&mut g, rng.range(-1000, 1000));
    }
    let n = rng.range(1, 3);
    let mut have_ins = false;
    for i in 0..n {
        let mut flags: u16 = 0;
        if rng.chance(50) {
            flags |= 1; // ARG_1_AND_2_ARE_WORDS
        }
        match rng.below(4) {
            1 => flags |= 1 << 3,
            2 => flags |= 1 << 6,
            3 => flags |= 1 << 7,
            _ => {}
        }
        if i + 1 < n {
            flags |= 1 << 5;
        }
        if rng.chance(20) {
            flags |= 1 << 8;
            have_ins = true;
        }
        be16(&mut g, i64::from(flags));
        be16(&mut g, rng.below(num_glyphs.max(1) as u64) as i64);
        let args = if flags & 1 != 0 { 4 } else { 2 };
        g.extend(rng.bytes(args));
        let scale = if flags & (1 << 3) != 0 {
            2
        } else if flags & (1 << 6) != 0 {
            4
        } else if flags & (1 << 7) != 0 {
            8
        } else {
            0
        };
        g.extend(rng.bytes(scale));
    }
    if have_ins {
        let ins = rng.range(0, 10) as usize;
        be16(&mut g, ins as i64);
        g.extend(rng.bytes(ins));
    }
    g
}

fn synth_glyf_font(rng: &mut Rng) -> Tables {
    let num_glyphs = rng.range(1, 24) as usize;
    let long_loca = rng.chance(40);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    let mut xmins = Vec::new();
    for _ in 0..num_glyphs {
        if long_loca {
            be32(&mut loca, glyf.len() as u32);
        } else {
            be16(&mut loca, (glyf.len() / 2) as i64);
        }
        let g = match rng.below(10) {
            0 => Vec::new(),
            1 => {
                let mut g = Vec::new();
                be16(&mut g, 0);
                for _ in 0..4 {
                    be16(
                        &mut g,
                        if rng.chance(50) {
                            0
                        } else {
                            rng.range(-10, 10)
                        },
                    );
                }
                g
            }
            2 | 3 => synth_composite_glyph(rng, num_glyphs),
            _ => synth_simple_glyph(rng),
        };
        xmins.push(if g.len() >= 4 {
            rd16(&g, 2).unwrap()
        } else {
            0
        });
        glyf.extend_from_slice(&g);
        if long_loca {
            if rng.chance(70) {
                pad4(&mut glyf);
            }
        } else if glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    if long_loca {
        be32(&mut loca, glyf.len() as u32);
    } else {
        be16(&mut loca, (glyf.len() / 2) as i64);
    }

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = long_loca as u8;
    if rng.chance(10) {
        head[16] = rng.next() as u8;
    }

    let num_hmetrics = rng.range(1, num_glyphs as i64) as usize;
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&(num_hmetrics as u16).to_be_bytes());
    let mut hmtx = Vec::new();
    for (i, xmin) in xmins.iter().enumerate() {
        let lsb = if rng.chance(85) {
            i64::from(*xmin as i16)
        } else {
            rng.range(-50, 50)
        };
        if i < num_hmetrics {
            be16(&mut hmtx, rng.range(0, 2000));
        }
        be16(&mut hmtx, lsb);
    }
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&(num_glyphs as u16).to_be_bytes());

    let mut tables: Tables = vec![
        (tag(b"head"), head),
        (tag(b"hhea"), hhea),
        (tag(b"hmtx"), hmtx),
        (tag(b"maxp"), maxp),
        (tag(b"glyf"), glyf),
        (tag(b"loca"), loca),
    ];
    let len = rng.below(40) as usize;
    for t in [
        b"cvt ", b"fpgm", b"prep", b"OS/2", b"name", b"cmap", b"post",
    ] {
        if rng.chance(30) {
            tables.push((tag(t), rng.bytes(len)));
        }
    }
    if rng.chance(10) {
        tables.push((tag(b"DSIG"), vec![0, 0, 0, 1, 0, 0, 0, 0]));
    }
    if rng.chance(10) {
        tables.push((tag(b"zZz!"), {
            let n = rng.below(16) as usize;
            rng.bytes(n)
        }));
    }
    if rng.chance(5) {
        // A table whose tag collides with the encoder's internal tags for
        // transformed tables.
        let t = if rng.chance(50) {
            tag(b"glyf") ^ 0x8080_8080
        } else {
            tag(b"cmap") ^ 0x8080_8080
        };
        tables.push((t, {
            let n = rng.below(16) as usize;
            rng.bytes(n)
        }));
    }
    tables
}

fn synth_cff_font(rng: &mut Rng) -> Tables {
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    let mut t = vec![
        (tag(b"head"), head),
        (tag(b"CFF "), {
            let n = rng.range(4, 64) as usize;
            rng.bytes(n)
        }),
    ];
    if rng.chance(50) {
        let mut hhea = vec![0u8; 36];
        hhea[35] = 1;
        t.push((tag(b"hhea"), hhea));
        t.push((tag(b"hmtx"), rng.bytes(8)));
    }
    t
}

// ---------------------------------------------------------------------------
// Corpus writing
// ---------------------------------------------------------------------------

struct Corpus {
    dir: PathBuf,
    n: usize,
}

impl Corpus {
    fn new(root: &Path, name: &str) -> Corpus {
        let dir = root.join(name);
        fs::create_dir_all(&dir).unwrap();
        Corpus { dir, n: 0 }
    }
    fn add(&mut self, data: &[u8]) {
        let h = data.iter().fold(1469598103934665603u64, |h, &b| {
            (h ^ u64::from(b)).wrapping_mul(1099511628211)
        });
        fs::write(self.dir.join(format!("{h:016x}")), data).unwrap();
        self.n += 1;
    }
}

fn encode_input(font: &[u8], quality: u8, flags: u8, meta: &[u8], size_mode: u8) -> Vec<u8> {
    let mut v = vec![quality, flags, meta.len() as u8, size_mode];
    v.extend_from_slice(meta);
    v.extend_from_slice(font);
    v
}

fn to_woff2(font: &[u8], quality: i32, transforms: bool, meta: &[u8]) -> Option<Vec<u8>> {
    woff2::convert_ttf_to_woff2(
        font,
        &woff2::Woff2Params {
            extended_metadata: meta.to_vec(),
            brotli_quality: quality,
            allow_transforms: transforms,
        },
    )
}

extern "C" {
    fn BrotliDecoderDecompress(
        encoded_size: usize,
        encoded_buffer: *const u8,
        decoded_size: *mut usize,
        decoded_buffer: *mut u8,
    ) -> c_int;
}

/// WOFF2 -> "structured" form (compressed block stored uncompressed).
fn to_structured(woff2: &[u8]) -> Option<Vec<u8>> {
    use woff2::buffer::Buffer;
    use woff2::table_tags::{GLYF_TABLE_TAG, KNOWN_TAGS, LOCA_TABLE_TAG};
    use woff2::variable_length::{read_255_ushort, read_base128};
    let mut b = Buffer::new(woff2);
    b.skip(4);
    let flavor = b.read_u32()?;
    b.skip(4);
    let num_tables = b.read_u16()?;
    b.skip(6);
    let compressed_len = b.read_u32()? as usize;
    b.skip(24);
    let mut total = 0usize;
    for _ in 0..num_tables {
        let flags = b.read_u8()?;
        let t = if flags & 0x3f == 0x3f {
            b.read_u32()?
        } else {
            KNOWN_TAGS[usize::from(flags & 0x3f)]
        };
        let v = (flags >> 6) & 3;
        let orig = read_base128(&mut b)? as usize;
        let xf = if t == GLYF_TABLE_TAG || t == LOCA_TABLE_TAG {
            v == 0
        } else {
            v != 0
        };
        total += if xf {
            read_base128(&mut b)? as usize
        } else {
            orig
        };
    }
    if flavor == woff2::common::TTC_FONT_FLAVOR {
        b.skip(4);
        let nf = read_255_ushort(&mut b)?;
        for _ in 0..nf {
            let nt = read_255_ushort(&mut b)?;
            b.skip(4);
            for _ in 0..nt {
                read_255_ushort(&mut b)?;
            }
        }
    }
    let off = b.offset();
    let mut payload = vec![0u8; total];
    let mut size = total;
    let src = woff2.get(off..off + compressed_len)?;
    let r = unsafe {
        BrotliDecoderDecompress(src.len(), src.as_ptr(), &mut size, payload.as_mut_ptr())
    };
    if r != 1 || size != total {
        return None;
    }
    let mut out = woff2[..off].to_vec();
    out[14..16].copy_from_slice(&[0, 0]); // knobs
    out.extend_from_slice(&payload);
    Some(out)
}

// ---------------------------------------------------------------------------
// Targeted seeds for paths the reference encoder never produces
// ---------------------------------------------------------------------------

extern "C" {
    fn BrotliEncoderCompress(
        quality: c_int,
        lgwin: c_int,
        mode: c_int,
        input_size: usize,
        input_buffer: *const u8,
        encoded_size: *mut usize,
        encoded_buffer: *mut u8,
    ) -> c_int;
}

fn brotli(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8; data.len() + data.len() / 2 + 1024];
    let mut size = out.len();
    let ok = unsafe {
        BrotliEncoderCompress(
            1,
            22,
            2,
            data.len(),
            data.as_ptr(),
            &mut size,
            out.as_mut_ptr(),
        )
    };
    assert!(ok != 0);
    out.truncate(size);
    out
}

fn write_base128(v: &mut Vec<u8>, x: u32) {
    let mut tmp = [0u8; 10];
    let n = woff2::variable_length::encode_base128(x as usize, &mut tmp);
    v.extend_from_slice(&tmp[..n]);
}

/// A WOFF2 file broken into its parts (payload uncompressed).
struct W2 {
    header: Vec<u8>,
    entries: Vec<(u8, u32, u32, Option<u32>)>, // flags, tag, orig, transform len
    tables: Vec<Vec<u8>>,                      // per entry payload
    ttc_dir: Vec<u8>,
}

fn parse_w2(woff2: &[u8]) -> Option<W2> {
    use woff2::buffer::Buffer;
    use woff2::table_tags::{GLYF_TABLE_TAG, KNOWN_TAGS, LOCA_TABLE_TAG};
    use woff2::variable_length::{read_255_ushort, read_base128};
    let structured = to_structured(woff2)?;
    let mut b = Buffer::new(&structured);
    b.skip(12);
    let num_tables = b.read_u16()?;
    let flavor = rd32(&structured, 4)?;
    b.set_offset(48);
    let mut entries = Vec::new();
    for _ in 0..num_tables {
        let flags = b.read_u8()?;
        let t = if flags & 0x3f == 0x3f {
            b.read_u32()?
        } else {
            KNOWN_TAGS[usize::from(flags & 0x3f)]
        };
        let v = (flags >> 6) & 3;
        let orig = read_base128(&mut b)?;
        let xf = if t == GLYF_TABLE_TAG || t == LOCA_TABLE_TAG {
            v == 0
        } else {
            v != 0
        };
        let xl = if xf {
            Some(read_base128(&mut b)?)
        } else {
            None
        };
        entries.push((flags, t, orig, xl));
    }
    let dir_start = b.offset();
    if flavor == woff2::common::TTC_FONT_FLAVOR {
        b.skip(4);
        let nf = read_255_ushort(&mut b)?;
        for _ in 0..nf {
            let nt = read_255_ushort(&mut b)?;
            b.skip(4);
            for _ in 0..nt {
                read_255_ushort(&mut b)?;
            }
        }
    }
    let ttc_dir = structured[dir_start..b.offset()].to_vec();
    let mut payload = &structured[b.offset()..];
    let mut tables = Vec::new();
    for e in &entries {
        let n = e.3.unwrap_or(e.2) as usize;
        tables.push(payload.get(..n)?.to_vec());
        payload = &payload[n..];
    }
    Some(W2 {
        header: structured[..48].to_vec(),
        entries,
        tables,
        ttc_dir,
    })
}

impl W2 {
    /// Serializes; returns (structured form, real WOFF2).
    fn write(&self) -> (Vec<u8>, Vec<u8>) {
        let mut dir = Vec::new();
        for (flags, t, orig, xl) in &self.entries {
            dir.push(*flags);
            if flags & 0x3f == 0x3f {
                be32(&mut dir, *t);
            }
            write_base128(&mut dir, *orig);
            if let Some(x) = xl {
                write_base128(&mut dir, *x);
            }
        }
        dir.extend_from_slice(&self.ttc_dir);
        let payload: Vec<u8> = self.tables.concat();
        let mut header = self.header.clone();
        header[12..14].copy_from_slice(&(self.entries.len() as u16).to_be_bytes());
        header[14..16].copy_from_slice(&[0, 0]);
        header[28..48].fill(0);
        let mut st = header.clone();
        st.extend_from_slice(&dir);
        st.extend_from_slice(&payload);
        let compressed = brotli(&payload);
        let mut w = header;
        w.extend_from_slice(&dir);
        w.extend_from_slice(&compressed);
        pad4(&mut w);
        let len = w.len() as u32;
        w[8..12].copy_from_slice(&len.to_be_bytes());
        w[20..24].copy_from_slice(&(compressed.len() as u32).to_be_bytes());
        (st, w)
    }
}

/// Applies the (optional, never produced by the reference encoder) hmtx
/// transform from section 5.4 of the WOFF2 spec.
fn transform_hmtx(w: &mut W2, flags: u8) -> Option<()> {
    let tag_of = |t: &[u8; 4]| tag(t);
    let hhea = w.entries.iter().position(|e| e.1 == tag_of(b"hhea"))?;
    let hmtx = w.entries.iter().position(|e| e.1 == tag_of(b"hmtx"))?;
    let glyf = w.entries.iter().position(|e| e.1 == tag_of(b"glyf"))?;
    let nh = rd16(&w.tables[hhea], 34)? as usize;
    let ng = rd16(&w.tables[glyf], 4)? as usize;
    let old = &w.tables[hmtx];
    let mut t = vec![flags];
    for i in 0..nh {
        t.extend_from_slice(old.get(4 * i..4 * i + 2)?);
    }
    if flags & 1 == 0 {
        for i in 0..nh {
            t.extend_from_slice(old.get(4 * i + 2..4 * i + 4)?);
        }
    }
    if flags & 2 == 0 {
        for i in nh..ng {
            t.extend_from_slice(old.get(4 * nh + 2 * (i - nh)..4 * nh + 2 * (i - nh) + 2)?);
        }
    }
    let e = &mut w.entries[hmtx];
    *e = (0x40 | (e.0 & 0x3f), e.1, e.2, Some(t.len() as u32));
    w.tables[hmtx] = t;
    Some(())
}

/// Appends a private data block to a WOFF2 file.
fn with_private_block(woff2: &[u8], data: &[u8]) -> Vec<u8> {
    let mut w = woff2.to_vec();
    pad4(&mut w);
    let off = w.len() as u32;
    w.extend_from_slice(data);
    let len = w.len() as u32;
    w[8..12].copy_from_slice(&len.to_be_bytes());
    w[40..44].copy_from_slice(&off.to_be_bytes());
    w[44..48].copy_from_slice(&(data.len() as u32).to_be_bytes());
    w
}

/// A font with many 18-byte glyphs: its glyf fits a short loca, but the
/// normalized (4-byte aligned) glyf does not, so the encoder has to retry
/// with a long loca.
fn big_short_loca_font(num_glyphs: usize) -> Tables {
    let mut g = Vec::new();
    be16(&mut g, 1);
    for v in [0, 0, 10, 0] {
        be16(&mut g, v);
    }
    be16(&mut g, 1); // endPts: 2 points
    be16(&mut g, 0); // no instructions
    g.extend_from_slice(&[0x33 | 0x08, 1]); // on curve, x short positive, y same; repeat
    g.extend_from_slice(&[5, 5]);
    assert_eq!(g.len(), 18);
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for _ in 0..num_glyphs {
        be16(&mut loca, (glyf.len() / 2) as i64);
        glyf.extend_from_slice(&g);
    }
    be16(&mut loca, (glyf.len() / 2) as i64);
    let mut head = vec![0u8; 54];
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[4..6].copy_from_slice(&(num_glyphs as u16).to_be_bytes());
    vec![
        (tag(b"head"), head),
        (tag(b"hhea"), hhea),
        (tag(b"hmtx"), vec![0, 100, 0, 0]),
        (tag(b"maxp"), maxp),
        (tag(b"glyf"), glyf),
        (tag(b"loca"), loca),
    ]
}

/// A simple glyph whose normalized encoding is ~12% larger than the
/// original: all flags equal (one repeated flag byte) with long coordinates,
/// while the normalized flags alternate between short and long x.
fn inflating_glyph(n: usize, trailing: usize) -> Vec<u8> {
    let mut g = Vec::new();
    be16(&mut g, 1);
    for v in [0, 0, 1000, 1000] {
        be16(&mut g, v);
    }
    be16(&mut g, n as i64 - 1);
    be16(&mut g, 0);
    let mut left = n;
    while left > 0 {
        let run = left.min(256);
        if run > 1 {
            g.extend_from_slice(&[0x08, (run - 1) as u8]);
        } else {
            g.push(0);
        }
        left -= run;
    }
    for i in 0..n {
        be16(&mut g, if i % 2 == 0 { 1 } else { 300 });
    }
    for _ in 0..n {
        be16(&mut g, 300);
    }
    g.extend(std::iter::repeat(0).take(trailing));
    g
}

fn small_glyph() -> Vec<u8> {
    let mut g = Vec::new();
    be16(&mut g, 1);
    for v in [0, 0, 5, 0] {
        be16(&mut g, v);
    }
    be16(&mut g, 0);
    be16(&mut g, 0);
    g.extend_from_slice(&[0x33, 5]);
    g
}

fn font_from_glyphs(glyphs: &[Vec<u8>], long_loca: bool) -> Tables {
    let mut glyf = Vec::new();
    let mut loca = Vec::new();
    for g in glyphs {
        if long_loca {
            be32(&mut loca, glyf.len() as u32);
        } else {
            be16(&mut loca, (glyf.len() / 2) as i64);
        }
        glyf.extend_from_slice(g);
        if !long_loca && glyf.len() % 2 != 0 {
            glyf.push(0);
        }
    }
    if long_loca {
        be32(&mut loca, glyf.len() as u32);
    } else {
        be16(&mut loca, (glyf.len() / 2) as i64);
    }
    let mut head = vec![0u8; 54];
    head[51] = long_loca as u8;
    let mut hhea = vec![0u8; 36];
    hhea[35] = 1;
    vec![
        (tag(b"head"), head),
        (tag(b"hhea"), hhea),
        (tag(b"hmtx"), vec![0, 100, 0, 0]),
        (tag(b"glyf"), glyf),
        (tag(b"loca"), loca),
    ]
}

/// Builds a "structured" WOFF2 (see the decode_structured fuzz target) from
/// (flag byte, tag, origLength, transformLength, data) entries.
fn make_structured(flavor: u32, entries: &[(u8, u32, u32, Option<u32>, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    be32(&mut out, 0x774F_4632);
    be32(&mut out, flavor);
    be32(&mut out, 0); // length, fixed up by the harness
    be16(&mut out, entries.len() as i64);
    be16(&mut out, 0); // knobs
    be32(&mut out, 0); // totalSfntSize
    be32(&mut out, 0); // totalCompressedSize, fixed up by the harness
    out.extend_from_slice(&[0; 24]);
    for (flags, t, orig, xl, _) in entries {
        out.push(*flags);
        if flags & 0x3f == 0x3f {
            be32(&mut out, *t);
        }
        write_base128(&mut out, *orig);
        if let Some(x) = xl {
            write_base128(&mut out, *x);
        }
    }
    for e in entries {
        out.extend_from_slice(&e.4);
    }
    out
}

/// A transformed glyf table with the given streams.
fn transformed_glyf(num_glyphs: u16, index_format: u16, streams: [&[u8]; 7]) -> Vec<u8> {
    let mut g = Vec::new();
    be16(&mut g, 0);
    be16(&mut g, 0);
    be16(&mut g, i64::from(num_glyphs));
    be16(&mut g, i64::from(index_format));
    for s in &streams {
        be32(&mut g, s.len() as u32);
    }
    for s in &streams {
        g.extend_from_slice(s);
    }
    g
}

fn glyf_woff2_entries(glyf: Vec<u8>, num_glyphs: u32) -> Vec<(u8, u32, u32, Option<u32>, Vec<u8>)> {
    let mut head = vec![0u8; 54];
    head[51] = 1;
    let glen = glyf.len() as u32;
    let mut entries = vec![
        (1, tag(b"head"), 54, None, head),
        (10, tag(b"glyf"), 0, Some(glen), glyf),
        (11, tag(b"loca"), 4 * (num_glyphs + 1), Some(0), Vec::new()),
    ];
    if glen > 4096 {
        // Incompressible filler, so that the decoder's check against
        // implausible compression ratios (> 100) passes.
        let mut rng = Rng(u64::from(glen) | 1);
        let filler = rng.bytes(glen as usize / 64 + 4096);
        entries.push((0x3f, tag(b"zzzz"), filler.len() as u32, None, filler));
    }
    entries
}

fn collect_fonts(paths: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for p in paths {
        let p = PathBuf::from(p);
        if p.is_dir() {
            let mut stack = vec![p];
            while let Some(d) = stack.pop() {
                for e in fs::read_dir(&d).into_iter().flatten().flatten() {
                    let path = e.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                        if ["ttf", "otf", "ttc", "otc"].contains(&ext.to_ascii_lowercase().as_str())
                        {
                            out.push(path);
                        }
                    }
                }
            }
        } else {
            out.push(p);
        }
    }
    out.sort();
    out
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let regression = if let Some(i) = args.iter().position(|a| a == "--regression") {
        args.remove(i);
        true
    } else {
        false
    };
    if args.is_empty() {
        eprintln!("usage: woff2-seedgen [--regression] <out-dir> [fonts or dirs...]");
        std::process::exit(2);
    }
    let root = PathBuf::from(&args[0]);
    let mut encode = Corpus::new(&root, "encode");
    let mut decode = Corpus::new(&root, "decode");
    let mut structured = Corpus::new(&root, "decode_structured");
    let mut common = Corpus::new(&root, "common");
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);

    // Fonts used as seeds (sfnt bytes).
    let mut seeds: Vec<Vec<u8>> = Vec::new();

    // Synthetic single fonts.
    let mut synth: Vec<Tables> = Vec::new();
    for i in 0..160 {
        let t = if i % 10 == 9 {
            synth_cff_font(&mut rng)
        } else {
            synth_glyf_font(&mut rng)
        };
        let flavor = if i % 10 == 9 {
            tag(b"OTTO")
        } else {
            0x0001_0000
        };
        seeds.push(write_sfnt(flavor, &t));
        synth.push(t);
    }

    // Synthetic collections, with various kinds of table sharing.
    for i in 0..60 {
        let a = &synth[rng.below(synth.len() as u64) as usize];
        let b = &synth[rng.below(synth.len() as u64) as usize];
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut fa = Vec::new();
        for (t, d) in a {
            fa.push((*t, blobs.len()));
            blobs.push(d.clone());
        }
        let mut fb = Vec::new();
        let share = rng.below(4); // 0: nothing, 1: glyf/loca/hmtx, 2: head, 3: all
        for (t, d) in b {
            let shared = match share {
                1 => [b"glyf", b"loca", b"hmtx"].iter().any(|x| tag(x) == *t),
                2 => *t == tag(b"head"),
                3 => true,
                _ => false,
            };
            match fa.iter().find(|x| x.0 == *t) {
                Some(&(_, bi)) if shared => fb.push((*t, bi)),
                _ => {
                    fb.push((*t, blobs.len()));
                    blobs.push(d.clone());
                }
            }
        }
        let mut fonts = vec![(0x0001_0000, fa), (0x0001_0000, fb)];
        if i % 7 == 0 {
            fonts.push(fonts[0].clone()); // the very same font twice
        }
        if i % 11 == 0 {
            fonts.truncate(1);
        }
        let version = if rng.chance(50) {
            0x0001_0000
        } else {
            0x0002_0000
        };
        seeds.push(write_ttc(version, &fonts, &blobs));
    }

    // Real fonts given on the command line: minified.
    let fonts = collect_fonts(&args[1..]);
    for path in &fonts {
        let data = match fs::read(path) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let parsed = parse_fonts(&data);
        for (k, (flavor, tables)) in parsed.iter().enumerate().take(2) {
            for n in [3usize, 12, 40] {
                if let Some(m) = minify(tables, n, n == 12) {
                    let s = write_sfnt(*flavor, &m);
                    if s.len() <= 48 * 1024 {
                        seeds.push(s);
                    }
                }
            }
            if k == 0 && data.len() <= 16 * 1024 {
                seeds.push(data.clone());
            }
        }
        if parsed.len() >= 2 {
            // A minified collection of the first two fonts, sharing whatever
            // identical tables they have.
            let mut blobs: Vec<Vec<u8>> = Vec::new();
            let mut fonts_out = Vec::new();
            for (flavor, tables) in parsed.iter().take(2) {
                if let Some(m) = minify(tables, 8, false) {
                    let mut f = Vec::new();
                    for (t, d) in m {
                        let bi = match blobs.iter().position(|b| *b == d) {
                            Some(bi) => bi,
                            None => {
                                blobs.push(d);
                                blobs.len() - 1
                            }
                        };
                        f.push((t, bi));
                    }
                    fonts_out.push((*flavor, f));
                }
            }
            seeds.push(write_ttc(0x0001_0000, &fonts_out, &blobs));
        }
    }

    // Encoder seeds.
    for (i, s) in seeds.iter().enumerate() {
        let meta: &[u8] = b"<?xml version=\"1.0\"?><metadata version=\"1.0\"/>";
        encode.add(&encode_input(s, (i % 6) as u8, 1, &[], 0));
        encode.add(&encode_input(s, 1, 0, &[], (i % 4) as u8));
        if i % 5 == 0 {
            encode.add(&encode_input(s, 2, 5, meta, 3));
        }
        if i % 9 == 0 {
            encode.add(&encode_input(s, 11, 3, &[], 0));
        }
    }

    // Decoder seeds (WOFF2 files and their structured form).
    for (i, s) in seeds.iter().enumerate() {
        for (q, xf, meta) in [
            (1, true, &b""[..]),
            (4, false, &b""[..]),
            (11, true, &b"<m/>"[..]),
        ] {
            if q == 11 && i % 3 != 0 {
                continue;
            }
            if let Some(w) = to_woff2(s, q, xf, meta) {
                decode.add(&w);
                if let Some(st) = to_structured(&w) {
                    structured.add(&st);
                }
            }
        }
    }

    for _ in 0..64 {
        let n = rng.below(32) as usize;
        common.add(&rng.bytes(n));
    }

    // Targeted seeds.
    let mut targeted = 0;
    for (i, s) in seeds.iter().enumerate().filter(|(i, _)| i % 2 == 0) {
        let w = match to_woff2(s, 1, true, &[]) {
            Some(w) => w,
            None => continue,
        };
        // Transformed hmtx (all three valid flag combinations).
        if let Some(parsed) = parse_w2(&w) {
            for flags in [1u8, 2, 3] {
                let mut p = parse_w2(&w).unwrap();
                if transform_hmtx(&mut p, flags).is_some() {
                    let (st, real) = p.write();
                    structured.add(&st);
                    decode.add(&real);
                    targeted += 1;
                }
            }
            drop(parsed);
        }
        // Private data block.
        if i % 4 == 0 {
            decode.add(&with_private_block(&w, b"private data"));
            targeted += 1;
        }
    }
    // Short loca that has to become long during normalization, alone and in
    // a collection where the second font reuses the first font's 'head'.
    let big = big_short_loca_font(7000);
    encode.add(&encode_input(&write_sfnt(0x0001_0000, &big), 1, 1, &[], 0));
    encode.add(&encode_input(&write_sfnt(0x0001_0000, &big), 1, 0, &[], 0));
    let small = &font_from_glyphs(&[small_glyph()], false);
    let mut blobs: Vec<Vec<u8>> = small.iter().map(|t| t.1.clone()).collect();
    let fa: Vec<(u32, usize)> = small.iter().enumerate().map(|(i, t)| (t.0, i)).collect();
    let head_idx = fa.iter().find(|x| x.0 == tag(b"head")).unwrap().1;
    let mut fb = vec![(tag(b"head"), head_idx)];
    for (t, d) in &big {
        if *t != tag(b"head") {
            fb.push((*t, blobs.len()));
            blobs.push(d.clone());
        }
    }
    let ttc = write_ttc(0x0001_0000, &[(0x0001_0000, fa), (0x0001_0000, fb)], &blobs);
    encode.add(&encode_input(&ttc, 1, 1, &[], 0));
    targeted += 3;

    // Normalization making glyphs bigger than the budget (StoreGlyph
    // failures), and near-misses around the end of the normalized buffer
    // (Round4 overshooting it), followed by empty/non-empty glyphs.
    for n in (40..400).step_by(7) {
        for trailing in 0..4 {
            let composite = synth_composite_glyph(&mut rng, 2);
            for (k, tail) in [
                vec![],
                vec![Vec::new(), small_glyph()],
                vec![small_glyph()],
                vec![composite],
            ]
            .into_iter()
            .enumerate()
            {
                let mut glyphs = vec![inflating_glyph(n, trailing)];
                glyphs.extend(tail);
                let long = (n + trailing + k) % 2 == 0;
                let f = write_sfnt(0x0001_0000, &font_from_glyphs(&glyphs, long));
                encode.add(&encode_input(&f, 0, 1, &[], 0));
                targeted += 1;
            }
        }
    }

    // An empty loca and glyf.
    let mut empty = font_from_glyphs(&[], false);
    for t in empty.iter_mut() {
        if t.0 == tag(b"loca") || t.0 == tag(b"glyf") {
            t.1.clear();
        }
    }
    encode.add(&encode_input(
        &write_sfnt(0x0001_0000, &empty),
        0,
        1,
        &[],
        0,
    ));

    // A collection where the second font stores a "transformed" table that it
    // shares with the first font (FUZZ-REFERENCE-PATCH(3)).
    {
        let xcmap = tag(b"cmap") ^ 0x8080_8080;
        let head = synth[0]
            .iter()
            .find(|t| t.0 == tag(b"head"))
            .unwrap()
            .1
            .clone();
        let blobs = vec![
            head.clone(),
            b"cmap0123".to_vec(),
            b"xcmap000".to_vec(),
            head,
            b"cmap4567".to_vec(),
        ];
        for shared in [true, false] {
            let f0 = vec![(tag(b"head"), 0), (tag(b"cmap"), 1), (xcmap, 2)];
            let f1 = vec![
                (tag(b"head"), 3),
                (tag(b"cmap"), 4),
                (xcmap, if shared { 2 } else { 1 }),
            ];
            let ttc = write_ttc(0x0001_0000, &[(0x0001_0000, f0), (0x0001_0000, f1)], &blobs);
            encode.add(&encode_input(&ttc, 1, 1, &[], 0));
            encode.add(&encode_input(&ttc, 1, 0, &[], 0));
        }
    }

    // Collections with ~253 tables per font: the precomputed length counts
    // the transformed glyf/loca tables in numTables, the written data does
    // not.
    for extra_tables in [248usize, 249, 250, 251, 252] {
        let base = font_from_glyphs(&[small_glyph()], true);
        let mut blobs: Vec<Vec<u8>> = Vec::new();
        let mut fonts_out = Vec::new();
        for _ in 0..2 {
            let mut f = Vec::new();
            for (t, d) in &base {
                f.push((*t, blobs.len()));
                blobs.push(d.clone());
            }
            for i in 0..extra_tables {
                f.push((tag(b"Z000") + i as u32, blobs.len()));
                blobs.push(vec![i as u8; 4]);
            }
            fonts_out.push((0x0001_0000, f));
        }
        let ttc = write_ttc(0x0001_0000, &fonts_out, &blobs);
        encode.add(&encode_input(&ttc, 0, 1, &[], 0));
        encode.add(&encode_input(&ttc, 0, 0, &[], 3));
        targeted += 2;
    }

    // Decoder: >65536 points, coordinate overflow, a too-short glyf
    // transform, and a composite glyph without its instructions.
    {
        let mut np = Vec::new();
        for _ in 0..2 {
            np.push(253);
            be16(&mut np, 40000);
        }
        let flags = vec![0u8; 80000];
        let mut glyphs = vec![1u8; 80000];
        glyphs.push(0);
        let g = transformed_glyf(
            1,
            1,
            [&[0, 2], &np, &flags, &glyphs, &[], &[0, 0, 0, 0], &[]],
        );
        structured.add(&make_structured(0x0001_0000, &glyf_woff2_entries(g, 1)));

        let mut np = vec![253];
        be16(&mut np, 40000);
        let flags = vec![124u8 + 3; 40000]; // dx = dy = +65535 per point
        let mut glyphs = Vec::new();
        for _ in 0..40000 {
            glyphs.extend_from_slice(&[0xff, 0xff, 0xff, 0xff]);
        }
        glyphs.push(0);
        let g = transformed_glyf(
            1,
            1,
            [&[0, 1], &np, &flags, &glyphs, &[], &[0, 0, 0, 0], &[]],
        );
        structured.add(&make_structured(0x0001_0000, &glyf_woff2_entries(g, 1)));

        // Transformed glyf shorter than its 36 byte header.
        let mut g = Vec::new();
        be16(&mut g, 0);
        be16(&mut g, 0);
        be16(&mut g, 1);
        be16(&mut g, 1);
        g.extend_from_slice(&[0; 12]);
        structured.add(&make_structured(0x0001_0000, &glyf_woff2_entries(g, 1)));

        // Composite glyph with WE_HAVE_INSTRUCTIONS but no instructions.
        let mut comp = Vec::new();
        be16(&mut comp, (1 << 8) | 1);
        comp.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mut bbox = vec![0x80, 0, 0, 0];
        bbox.extend_from_slice(&[0; 8]);
        let g = transformed_glyf(1, 1, [&[0xff, 0xff], &[], &[], &[5], &comp, &bbox, &[1]]);
        structured.add(&make_structured(0x0001_0000, &glyf_woff2_entries(g, 1)));
        targeted += 4;
    }
    // Exhausting the normalization budget exactly while writing the flags of
    // the second glyph (zero deltas, alternating on-curve bits). The exact
    // parameters are found by simulating the normalization.
    let normalized_size = |g: &[u8]| -> usize {
        let mut glyph = woff2::encode::glyph::Glyph::default();
        assert!(woff2::encode::glyph::read_glyph(g, &mut glyph));
        let mut buf = vec![0u8; 1 << 20];
        woff2::encode::glyph::store_glyph(&glyph, &mut buf).unwrap()
    };
    let mut found = 0;
    for n in 20..2000 {
        for f in [8usize, 20, 50, 100, 200] {
            let mut g = Vec::new();
            be16(&mut g, 1);
            for v in [0, 0, 0, 0] {
                be16(&mut g, v);
            }
            be16(&mut g, f as i64 - 1);
            be16(&mut g, 0);
            for i in 0..f {
                g.push(0x30 | (i % 2) as u8);
            }
            let g0 = inflating_glyph(n, 0);
            let glyf_len = g0.len() + g.len();
            let budget = (1.1 * glyf_len as f64 + 4.0) as usize;
            let s0 = (normalized_size(&g0) + 3) & !3;
            let s1 = normalized_size(&g);
            if s0 > budget || budget - s0 < 14 || budget - s0 >= s1 {
                continue;
            }
            found += 1;
            if found % 20 != 1 {
                continue;
            }
            let glyphs = vec![g0, g];
            let font = write_sfnt(0x0001_0000, &font_from_glyphs(&glyphs, true));
            encode.add(&encode_input(&font, 0, 1, &[], 0));
            targeted += 1;
        }
    }
    eprintln!("{found} budget-exhausting fonts");
    for n in (100..640).step_by(9) {
        for f in [20usize, 50, 100] {
            let mut g = Vec::new();
            be16(&mut g, 1);
            for v in [0, 0, 0, 0] {
                be16(&mut g, v);
            }
            be16(&mut g, f as i64 - 1);
            be16(&mut g, 0);
            for i in 0..f {
                g.push(0x30 | (i % 2) as u8);
            }
            let glyphs = vec![inflating_glyph(n, 0), g];
            let font = write_sfnt(0x0001_0000, &font_from_glyphs(&glyphs, true));
            encode.add(&encode_input(&font, 0, 1, &[], 0));
            targeted += 1;
        }
    }

    // Transformed hmtx tables that are empty or truncated.
    for (i, s) in seeds.iter().enumerate().filter(|(i, _)| i % 16 == 0) {
        if let Some(w) = to_woff2(s, 1, true, &[]) {
            for flags in [1u8, 2, 3] {
                if let Some(mut p) = parse_w2(&w) {
                    if transform_hmtx(&mut p, flags).is_some() {
                        let h = p.entries.iter().position(|e| e.1 == tag(b"hmtx")).unwrap();
                        for cut in [0usize, 1, 2, 3] {
                            let mut q = parse_w2(&w).unwrap();
                            transform_hmtx(&mut q, flags).unwrap();
                            let n = q.tables[h].len().saturating_sub(cut * (i % 5 + 1));
                            q.tables[h].truncate(if cut == 0 { 0 } else { n });
                            let e = &mut q.entries[h];
                            e.3 = Some(q.tables[h].len() as u32);
                            structured.add(&q.write().0);
                            targeted += 1;
                        }
                    }
                }
            }
        }
    }

    // All tables empty: nothing to decompress.
    structured.add(&make_structured(
        0x0001_0000,
        &[(0x3f, tag(b"zzzz"), 0, None, Vec::new())],
    ));
    structured.add(&make_structured(
        0x0001_0000,
        &[
            (1, tag(b"head"), 0, None, Vec::new()),
            (5, tag(b"name"), 0, None, Vec::new()),
        ],
    ));

    // A font with only a 'head' table (searchRange 0), with and without DSIG.
    let head = font_from_glyphs(&[], false)[0].clone();
    encode.add(&encode_input(
        &write_sfnt(0x0001_0000, &vec![head.clone()]),
        0,
        1,
        &[],
        0,
    ));
    let dsig = (tag(b"DSIG"), vec![0, 0, 0, 1, 0, 0, 0, 0]);
    encode.add(&encode_input(
        &write_sfnt(0x0001_0000, &vec![head, dsig]),
        0,
        1,
        &[],
        0,
    ));

    // Truncated WOFF2 files, with the length field matching the truncation,
    // so that every read of the header and directories can fail.
    for (i, s) in seeds
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 40 == 0 || *i == 170)
    {
        if let Some(w) = to_woff2(s, 1, i % 80 == 0, &[]) {
            for len in 0..w.len().min(160) {
                let mut t = w[..len].to_vec();
                if len >= 12 {
                    t[8..12].copy_from_slice(&(len as u32).to_be_bytes());
                }
                decode.add(&t);
                targeted += 1;
            }
        }
    }
    eprintln!("{targeted} targeted seeds");

    if regression {
        let mut renc = Corpus::new(&root, "regression_encode");
        let mut rdec = Corpus::new(&root, "regression_decode");
        for path in &fonts {
            if let Ok(data) = fs::read(path) {
                renc.add(&encode_input(&data, 11, 1, &[], 0));
                renc.add(&encode_input(&data, 5, 0, &[], 3));
                if let Some(w) = to_woff2(&data, 11, true, &[]) {
                    rdec.add(&w);
                }
            }
        }
        eprintln!("regression: {} encode, {} decode inputs", renc.n, rdec.n);
    }

    eprintln!(
        "{} seeds from {} font files: {} encode, {} decode, {} decode_structured, {} common",
        seeds.len(),
        fonts.len(),
        encode.n,
        decode.n,
        structured.n,
        common.n
    );
}
