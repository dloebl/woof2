//! Round-trip tests of the Rust API on a small synthetic font.
#![cfg(feature = "system-brotli")]

use woff2::{MemoryOut, StringOut, Woff2Out, Woff2Params};

fn be16(v: &mut Vec<u8>, x: i32) {
    v.extend_from_slice(&(x as u16).to_be_bytes());
}

/// Builds a minimal TrueType font with a simple, an empty and a composite
/// glyph.
fn test_font(long_loca: bool) -> Vec<u8> {
    // Glyph 0: a triangle with a couple of instructions.
    let mut g0 = Vec::new();
    be16(&mut g0, 1); // numberOfContours
    for v in [0, 0, 500, 700] {
        be16(&mut g0, v);
    }
    be16(&mut g0, 2); // endPtsOfContours[0]
    be16(&mut g0, 2); // instructionLength
    g0.extend_from_slice(&[0xb0, 0x00]);
    g0.extend_from_slice(&[0x01, 0x01, 0x01]); // flags: on curve, long coords
    for v in [0, 500, -250] {
        be16(&mut g0, v); // x deltas
    }
    for v in [0, 0, 700] {
        be16(&mut g0, v); // y deltas
    }
    // Glyph 1: empty.
    // Glyph 2: composite referencing glyph 0.
    let mut g2 = Vec::new();
    be16(&mut g2, -1);
    for v in [10, 10, 510, 710] {
        be16(&mut g2, v);
    }
    be16(&mut g2, 0x0002); // ARGS_ARE_XY_VALUES
    be16(&mut g2, 0); // glyphIndex
    g2.extend_from_slice(&[10, 10]);

    let mut glyf = Vec::new();
    let mut offsets = vec![0usize];
    for g in [&g0, &Vec::new(), &g2] {
        glyf.extend_from_slice(g);
        while glyf.len() % 4 != 0 {
            glyf.push(0);
        }
        offsets.push(glyf.len());
    }
    let mut loca = Vec::new();
    for o in &offsets {
        if long_loca {
            loca.extend_from_slice(&(*o as u32).to_be_bytes());
        } else {
            be16(&mut loca, (*o / 2) as i32);
        }
    }

    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[51] = long_loca as u8;
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
    let mut hmtx = Vec::new();
    for (aw, lsb) in [(600, 0), (300, 0), (620, 10)] {
        be16(&mut hmtx, aw);
        be16(&mut hmtx, lsb);
    }
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&3u16.to_be_bytes());

    let mut tables: Vec<(&[u8; 4], Vec<u8>)> = vec![
        (b"glyf", glyf),
        (b"head", head),
        (b"hhea", hhea),
        (b"hmtx", hmtx),
        (b"loca", loca),
        (b"maxp", maxp),
        (b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]),
    ];
    tables.sort_by_key(|t| *t.0);

    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    be16(&mut out, tables.len() as i32);
    out.extend_from_slice(&[0, 64, 0, 2, 0, 48]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in &tables {
        out.extend_from_slice(*tag);
        out.extend_from_slice(&woff2::common::compute_ulong_sum(data).to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += (data.len() + 3) & !3;
    }
    for (_, data) in &tables {
        out.extend_from_slice(data);
        while out.len() % 4 != 0 {
            out.push(0);
        }
    }
    out
}

fn decode(woff2: &[u8]) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let size = {
        let mut out = StringOut::new(&mut buf);
        if !woff2::convert_woff2_to_ttf(woff2, &mut out) {
            return None;
        }
        out.size()
    };
    buf.truncate(size);
    Some(buf)
}

#[test]
fn round_trip_is_stable() {
    for long_loca in [false, true] {
        for allow_transforms in [false, true] {
            let font = test_font(long_loca);
            let params = Woff2Params {
                allow_transforms,
                ..Default::default()
            };
            let w1 = woff2::convert_ttf_to_woff2(&font, &params).expect("encode");
            assert_eq!(&w1[0..4], b"wOF2");
            assert_eq!(woff2::compute_woff2_final_size(&w1), {
                // totalSfntSize: header + entries (DSIG removed) + tables.
                let t1 = decode(&w1).unwrap();
                t1.len()
            });
            let ttf1 = decode(&w1).expect("decode");
            // The decoded font is normalized: encoding it again and decoding
            // must reproduce it exactly.
            let w2 = woff2::convert_ttf_to_woff2(&ttf1, &params).expect("re-encode");
            let ttf2 = decode(&w2).expect("re-decode");
            assert_eq!(ttf1, ttf2);
            // The deprecated fixed-size API gives the same result.
            let mut fixed = vec![0u8; woff2::compute_woff2_final_size(&w1)];
            assert!(woff2::decode::convert_woff2_to_ttf_buf(&mut fixed, &w1));
            assert_eq!(fixed, ttf1);
            // DSIG is dropped, 'head' flags bit 11 is set.
            assert!(!ttf1.windows(4).any(|w| w == b"DSIG"));
        }
    }
}

#[test]
fn metadata_and_small_buffers() {
    let font = test_font(false);
    let params = Woff2Params {
        extended_metadata: b"<?xml version=\"1.0\"?><metadata version=\"1.0\"/>".to_vec(),
        brotli_quality: 5,
        allow_transforms: true,
    };
    let woff2 = woff2::convert_ttf_to_woff2(&font, &params).expect("encode");
    assert!(decode(&woff2).is_some());

    // A result buffer that is too small fails and does not write past
    // `result_length`.
    let mut buf = vec![0xAAu8; 200];
    let mut len = 100;
    assert!(!woff2::convert_ttf_to_woff2_with(
        &font,
        &mut buf,
        &mut len,
        &params,
        woff2::brotli::SYSTEM_COMPRESS
    ));
    assert!(buf[100..].iter().all(|&b| b == 0xAA));

    // Decoding into a too small fixed buffer fails.
    let mut small = vec![0u8; 16];
    let mut out = MemoryOut::new(&mut small);
    assert!(!woff2::convert_woff2_to_ttf(&woff2, &mut out));
}

#[test]
fn rejects_garbage() {
    assert_eq!(woff2::compute_woff2_final_size(b"wOF2"), 0);
    let mut v = Vec::new();
    let mut out = StringOut::new(&mut v);
    assert!(!woff2::convert_woff2_to_ttf(b"", &mut out));
    assert!(!woff2::convert_woff2_to_ttf(&[0u8; 100], &mut out));
    assert!(woff2::convert_ttf_to_woff2(b"", &Woff2Params::default()).is_none());
    assert!(woff2::convert_ttf_to_woff2(&[0u8; 100], &Woff2Params::default()).is_none());
}

/// A WOFF2Out that fails after a number of calls.
struct Failing {
    inner: Vec<u8>,
    calls: usize,
    fail_at: usize,
}

impl Woff2Out for Failing {
    fn write(&mut self, buf: &[u8]) -> bool {
        let off = self.inner.len();
        self.write_at(buf, off)
    }
    fn write_at(&mut self, buf: &[u8], offset: usize) -> bool {
        self.calls += 1;
        if self.calls == self.fail_at {
            return false;
        }
        if self.inner.len() < offset + buf.len() {
            self.inner.resize(offset + buf.len(), 0);
        }
        self.inner[offset..offset + buf.len()].copy_from_slice(buf);
        true
    }
    fn size(&mut self) -> usize {
        self.inner.len()
    }
}

#[test]
fn custom_output_failures_propagate() {
    let font = test_font(true);
    let woff2 = woff2::convert_ttf_to_woff2(&font, &Woff2Params::default()).unwrap();
    let mut ok = Failing {
        inner: Vec::new(),
        calls: 0,
        fail_at: 0,
    };
    assert!(woff2::convert_woff2_to_ttf(&woff2, &mut ok));
    assert_eq!(ok.inner, decode(&woff2).unwrap());
    for fail_at in 1..=ok.calls {
        let mut out = Failing {
            inner: Vec::new(),
            calls: 0,
            fail_at,
        };
        assert!(!woff2::convert_woff2_to_ttf(&woff2, &mut out));
        assert_eq!(
            out.calls, fail_at,
            "decoding must stop at the failing write"
        );
    }
}
