//! Differential testing of the Rust implementation (through the unchanged
//! C++ API in `cpp/`) against the original C++ implementation.
//!
//! Everything observable through the public API is compared: return values,
//! output buffers (including bytes that are *not* supposed to be written),
//! output-size out-parameters, and, for decoding into a user-defined
//! `WOFF2Out`, the complete sequence of virtual calls including injected
//! write failures and exceptions.

use std::os::raw::{c_int, c_uint, c_void};

#[repr(C)]
pub struct GlueBuf {
    data: *mut u8,
    len: usize,
}

extern "C" {
    fn free(p: *mut c_void);
}

impl GlueBuf {
    fn empty() -> Self {
        GlueBuf {
            data: std::ptr::null_mut(),
            len: 0,
        }
    }

    fn take(self) -> Vec<u8> {
        if self.data.is_null() {
            return Vec::new();
        }
        // SAFETY: allocated with malloc by the glue, `len` bytes valid.
        let v = unsafe { std::slice::from_raw_parts(self.data, self.len) }.to_vec();
        unsafe { free(self.data.cast()) };
        v
    }
}

macro_rules! glue_api {
    ($($name:ident : fn($($arg:ty),*) -> $ret:ty;)*) => {
        /// One implementation of the glue API.
        #[derive(Clone, Copy)]
        pub struct Api {
            pub name: &'static str,
            $(pub $name: unsafe extern "C" fn($($arg),*) -> $ret,)*
        }
    };
}

glue_api! {
    final_size: fn(*const u8, usize) -> usize;
    decode_string: fn(*const u8, usize, usize, c_int, usize, *mut GlueBuf, *mut usize) -> c_int;
    decode_memory: fn(*mut u8, usize, *const u8, usize) -> c_int;
    decode_trace: fn(*const u8, usize, i64, i64, usize, *mut GlueBuf, *mut GlueBuf, *mut c_int) -> c_int;
    max_compressed_size: fn(*const u8, usize, *const u8, usize, c_int) -> usize;
    compressed_buffer_size: fn(u32) -> u32;
    encode: fn(*const u8, usize, *mut u8, *mut usize, *const u8, usize, c_int, c_int, c_int) -> c_int;
    read_255_ushort: fn(*const u8, usize, usize, *mut usize, *mut c_uint) -> c_int;
    read_base128: fn(*const u8, usize, usize, *mut usize, *mut u32) -> c_int;
    write_255_ushort: fn(c_int, *mut u8) -> usize;
    store_255_ushort: fn(c_int, usize, *mut u8) -> usize;
    size_255_ushort: fn(u16) -> usize;
    base128_size: fn(usize) -> usize;
    store_base128: fn(usize, usize, *mut u8) -> usize;
    ulong_sum: fn(*const u8, usize) -> u32;
    collection_header_size: fn(u32, u32) -> usize;
    known_tags: fn() -> *const u32;
}

macro_rules! glue_decls {
    ($api:ident, $label:expr, $($name:ident = $sym:ident : fn($($arg:ty),*) -> $ret:ty;)*) => {
        extern "C" {
            $(fn $sym($(_: $arg),*) -> $ret;)*
        }
        pub const $api: Api = Api {
            name: $label,
            $($name: $sym,)*
        };
    };
}

glue_decls! {
    REF, "reference",
    final_size = ref_final_size: fn(*const u8, usize) -> usize;
    decode_string = ref_decode_string: fn(*const u8, usize, usize, c_int, usize, *mut GlueBuf, *mut usize) -> c_int;
    decode_memory = ref_decode_memory: fn(*mut u8, usize, *const u8, usize) -> c_int;
    decode_trace = ref_decode_trace: fn(*const u8, usize, i64, i64, usize, *mut GlueBuf, *mut GlueBuf, *mut c_int) -> c_int;
    max_compressed_size = ref_max_compressed_size: fn(*const u8, usize, *const u8, usize, c_int) -> usize;
    compressed_buffer_size = ref_compressed_buffer_size: fn(u32) -> u32;
    encode = ref_encode: fn(*const u8, usize, *mut u8, *mut usize, *const u8, usize, c_int, c_int, c_int) -> c_int;
    read_255_ushort = ref_read_255_ushort: fn(*const u8, usize, usize, *mut usize, *mut c_uint) -> c_int;
    read_base128 = ref_read_base128: fn(*const u8, usize, usize, *mut usize, *mut u32) -> c_int;
    write_255_ushort = ref_write_255_ushort: fn(c_int, *mut u8) -> usize;
    store_255_ushort = ref_store_255_ushort: fn(c_int, usize, *mut u8) -> usize;
    size_255_ushort = ref_size_255_ushort: fn(u16) -> usize;
    base128_size = ref_base128_size: fn(usize) -> usize;
    store_base128 = ref_store_base128: fn(usize, usize, *mut u8) -> usize;
    ulong_sum = ref_ulong_sum: fn(*const u8, usize) -> u32;
    collection_header_size = ref_collection_header_size: fn(u32, u32) -> usize;
    known_tags = ref_known_tags: fn() -> *const u32;
}

glue_decls! {
    NEW, "rust",
    final_size = new_final_size: fn(*const u8, usize) -> usize;
    decode_string = new_decode_string: fn(*const u8, usize, usize, c_int, usize, *mut GlueBuf, *mut usize) -> c_int;
    decode_memory = new_decode_memory: fn(*mut u8, usize, *const u8, usize) -> c_int;
    decode_trace = new_decode_trace: fn(*const u8, usize, i64, i64, usize, *mut GlueBuf, *mut GlueBuf, *mut c_int) -> c_int;
    max_compressed_size = new_max_compressed_size: fn(*const u8, usize, *const u8, usize, c_int) -> usize;
    compressed_buffer_size = new_compressed_buffer_size: fn(u32) -> u32;
    encode = new_encode: fn(*const u8, usize, *mut u8, *mut usize, *const u8, usize, c_int, c_int, c_int) -> c_int;
    read_255_ushort = new_read_255_ushort: fn(*const u8, usize, usize, *mut usize, *mut c_uint) -> c_int;
    read_base128 = new_read_base128: fn(*const u8, usize, usize, *mut usize, *mut u32) -> c_int;
    write_255_ushort = new_write_255_ushort: fn(c_int, *mut u8) -> usize;
    store_255_ushort = new_store_255_ushort: fn(c_int, usize, *mut u8) -> usize;
    size_255_ushort = new_size_255_ushort: fn(u16) -> usize;
    base128_size = new_base128_size: fn(usize) -> usize;
    store_base128 = new_store_base128: fn(usize, usize, *mut u8) -> usize;
    ulong_sum = new_ulong_sum: fn(*const u8, usize) -> u32;
    collection_header_size = new_collection_header_size: fn(u32, u32) -> usize;
    known_tags = new_known_tags: fn() -> *const u32;
}

/// Upper bound for buffers sized from untrusted header fields.
const MAX_BUF: usize = 1 << 23;

fn hash(data: &[u8]) -> u64 {
    let mut h: u64 = 1469598103934665603;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(1099511628211);
    }
    h
}

fn first_diff(a: &[u8], b: &[u8]) -> Option<usize> {
    if a == b {
        return None;
    }
    Some(
        a.iter()
            .zip(b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len())),
    )
}

fn check_bytes(what: &str, r: &[u8], n: &[u8]) {
    if let Some(at) = first_diff(r, n) {
        panic!(
            "DIFFERENCE in {what}: lengths reference={} rust={}, first difference at byte {at}",
            r.len(),
            n.len()
        );
    }
}

macro_rules! check_eq {
    ($what:expr, $r:expr, $n:expr) => {{
        let (r, n) = (&$r, &$n);
        if r != n {
            panic!("DIFFERENCE in {}: reference={:?} rust={:?}", $what, r, n);
        }
    }};
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
struct StringResult {
    ok: bool,
    contents: Vec<u8>,
    size: usize,
}

fn decode_string(api: &Api, data: &[u8], initial: usize, max: Option<usize>) -> StringResult {
    let mut buf = GlueBuf::empty();
    let mut size = 0usize;
    let ok = unsafe {
        (api.decode_string)(
            data.as_ptr(),
            data.len(),
            initial,
            max.is_some() as c_int,
            max.unwrap_or(0),
            &mut buf,
            &mut size,
        )
    } != 0;
    StringResult {
        ok,
        contents: buf.take(),
        size,
    }
}

fn diff_decode_string(data: &[u8], initial: usize, max: Option<usize>) -> StringResult {
    let r = decode_string(&REF, data, initial, max);
    let n = decode_string(&NEW, data, initial, max);
    let what = format!("ConvertWOFF2ToTTF(WOFF2StringOut initial={initial} max={max:?})");
    check_eq!(format!("{what} result"), r.ok, n.ok);
    check_eq!(format!("{what} Size()"), r.size, n.size);
    check_bytes(&format!("{what} output"), &r.contents, &n.contents);
    r
}

fn diff_decode_memory(data: &[u8], result_length: usize, fill: u8) {
    let mut r = vec![fill; result_length];
    let mut n = vec![fill; result_length];
    let ok_r = unsafe { (REF.decode_memory)(r.as_mut_ptr(), r.len(), data.as_ptr(), data.len()) };
    let ok_n = unsafe { (NEW.decode_memory)(n.as_mut_ptr(), n.len(), data.as_ptr(), data.len()) };
    let what = format!("ConvertWOFF2ToTTF(uint8_t*, {result_length}, ...)");
    check_eq!(format!("{what} result"), ok_r, ok_n);
    check_bytes(&format!("{what} output"), &r, &n);
}

struct TraceResult {
    ok: bool,
    threw: bool,
    trace: Vec<u8>,
    contents: Vec<u8>,
}

fn decode_trace(api: &Api, data: &[u8], fail_at: i64, throw_at: i64, max: usize) -> TraceResult {
    let mut trace = GlueBuf::empty();
    let mut contents = GlueBuf::empty();
    let mut threw: c_int = 0;
    let ok = unsafe {
        (api.decode_trace)(
            data.as_ptr(),
            data.len(),
            fail_at,
            throw_at,
            max,
            &mut trace,
            &mut contents,
            &mut threw,
        )
    } != 0;
    TraceResult {
        ok,
        threw: threw != 0,
        trace: trace.take(),
        contents: contents.take(),
    }
}

/// Each traced event is 1 + 3 * 8 bytes.
const EVENT_SIZE: usize = 25;

fn diff_decode_trace(data: &[u8], fail_at: i64, throw_at: i64, max: usize) -> usize {
    let r = decode_trace(&REF, data, fail_at, throw_at, max);
    let n = decode_trace(&NEW, data, fail_at, throw_at, max);
    let what = format!(
        "ConvertWOFF2ToTTF(custom WOFF2Out fail_at={fail_at} throw_at={throw_at} max={max})"
    );
    check_eq!(format!("{what} result"), r.ok, n.ok);
    check_eq!(format!("{what} exception"), r.threw, n.threw);
    if r.trace != n.trace {
        let at = first_diff(&r.trace, &n.trace).unwrap() / EVENT_SIZE;
        let ev = |t: &[u8]| -> Option<(char, u64, u64, u64)> {
            let e = t.get(at * EVENT_SIZE..(at + 1) * EVENT_SIZE)?;
            let w = |i: usize| u64::from_ne_bytes(e[1 + 8 * i..9 + 8 * i].try_into().unwrap());
            Some((e[0] as char, w(0), w(1), w(2)))
        };
        panic!(
            "DIFFERENCE in {what} call sequence: {} vs {} calls, first difference at call #{at}: reference={:?} rust={:?}",
            r.trace.len() / EVENT_SIZE,
            n.trace.len() / EVENT_SIZE,
            ev(&r.trace),
            ev(&n.trace)
        );
    }
    check_bytes(&format!("{what} output"), &r.contents, &n.contents);
    r.trace.len() / EVENT_SIZE
}

/// Compares every decoding entry point on `data`.
pub fn diff_decode(data: &[u8]) {
    let h = hash(data);

    let fs_r = unsafe { (REF.final_size)(data.as_ptr(), data.len()) };
    let fs_n = unsafe { (NEW.final_size)(data.as_ptr(), data.len()) };
    check_eq!("ComputeWOFF2FinalSize", fs_r, fs_n);

    // The woff2_decompress / fuzzer pattern, and an empty string.
    let full = diff_decode_string(data, fs_r.min(MAX_BUF), None);
    diff_decode_string(data, 0, None);

    // Output size limits that make the conversion fail at some point.
    if full.size > 0 {
        let s = full.size;
        diff_decode_string(data, 0, Some(s - 1 - (h as usize % s.min(64))));
        diff_decode_string(data, 0, Some((h as usize >> 8) % s));
    }
    diff_decode_string(data, 0, Some(30 * 1024 * 1024));

    // The deprecated fixed-buffer API.
    diff_decode_memory(data, fs_r.min(MAX_BUF), 0);
    if full.size > 0 {
        diff_decode_memory(data, full.size, 0xA5);
        diff_decode_memory(data, full.size - 1, 0xA5);
    }

    // A user-defined WOFF2Out: the complete call sequence must match,
    // including when a call fails or throws.
    let calls = diff_decode_trace(data, -1, -1, usize::MAX);
    if calls > 0 {
        let k = calls as i64;
        diff_decode_trace(data, 1 + (h % k as u64) as i64, -1, usize::MAX);
        diff_decode_trace(data, -1, 1 + ((h >> 16) % k as u64) as i64, usize::MAX);
        diff_decode_trace(data, k, -1, usize::MAX);
        diff_decode_trace(data, -1, k, usize::MAX);
    }
}

// ---------------------------------------------------------------------------
// Structured decoding: the compressed stream is stored uncompressed in the
// fuzzer input, so that mutations reach the font reconstruction code.
// ---------------------------------------------------------------------------

/// Parses the WOFF2 header and directories (permissively) and returns the
/// offset of the compressed data and the uncompressed size the directory
/// implies.
fn compressed_data_offset(data: &[u8]) -> Option<(usize, u64)> {
    use woff2::buffer::Buffer;
    use woff2::table_tags::{GLYF_TABLE_TAG, KNOWN_TAGS, LOCA_TABLE_TAG};
    use woff2::variable_length::{read_255_ushort, read_base128};

    fn skip(b: &mut Buffer<'_>, n: usize) -> Option<()> {
        b.skip(n).then_some(())
    }
    let mut b = Buffer::new(data);
    skip(&mut b, 4)?;
    let flavor = b.read_u32()?;
    skip(&mut b, 4)?;
    let num_tables = b.read_u16()?;
    skip(&mut b, 48 - 14)?;
    let mut total: u64 = 0;
    for _ in 0..num_tables {
        let flags = b.read_u8()?;
        let tag = if flags & 0x3f == 0x3f {
            b.read_u32()?
        } else {
            KNOWN_TAGS[usize::from(flags & 0x3f)]
        };
        let version = (flags >> 6) & 3;
        let orig = read_base128(&mut b)?;
        let transformed = if tag == GLYF_TABLE_TAG || tag == LOCA_TABLE_TAG {
            version == 0
        } else {
            version != 0
        };
        total += u64::from(if transformed {
            read_base128(&mut b)?
        } else {
            orig
        });
    }
    if flavor == woff2::common::TTC_FONT_FLAVOR {
        skip(&mut b, 4)?;
        let num_fonts = read_255_ushort(&mut b)?;
        for _ in 0..num_fonts {
            let n = read_255_ushort(&mut b)?;
            skip(&mut b, 4)?;
            for _ in 0..n {
                read_255_ushort(&mut b)?;
            }
        }
    }
    Some((b.offset(), total))
}

/// Turns a "structured" input (a WOFF2 file whose compressed data is stored
/// uncompressed) into a real WOFF2 file. The header's `reserved` field (which
/// decoders ignore) carries knobs for the fix-ups.
pub fn structured_to_woff2(input: &[u8]) -> Option<Vec<u8>> {
    let (offset, total) = compressed_data_offset(input)?;
    let knobs = u16::from_be_bytes([input[14], input[15]]);
    let mut payload = input[offset..].to_vec();
    if knobs & 1 == 0 && total <= MAX_BUF as u64 {
        // Make the payload exactly as long as the table directory says.
        payload.resize(total as usize, 0);
    }
    let quality = i32::from((knobs >> 1) & 1);
    let mut compressed = vec![0u8; payload.len() + payload.len() / 2 + 1024];
    let mut size = compressed.len();
    let ok = unsafe {
        BrotliEncoderCompress(
            quality,
            22,
            2,
            payload.len(),
            payload.as_ptr(),
            &mut size,
            compressed.as_mut_ptr(),
        )
    };
    if ok == 0 {
        return None;
    }
    compressed.truncate(size);
    let mut out = input[..offset].to_vec();
    out.extend_from_slice(&compressed);
    while out.len() % 4 != 0 {
        out.push(0);
    }
    let len = out.len() as u32;
    if knobs & 4 == 0 {
        out[8..12].copy_from_slice(&len.to_be_bytes());
    }
    out[20..24].copy_from_slice(&(size as u32).to_be_bytes());
    if knobs & 8 == 0 {
        // No metadata / private data blocks.
        out[28..48].fill(0);
    }
    Some(out)
}

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

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

/// Bytes beyond the claimed result length; must stay untouched.
const SLACK: usize = 1 << 16;
const FILL: u8 = 0xA5;

struct EncodeResult {
    ok: bool,
    result_length: usize,
    buf: Vec<u8>,
}

#[allow(clippy::too_many_arguments)]
fn encode(
    api: &Api,
    font: &[u8],
    claimed: usize,
    meta: &[u8],
    quality: i32,
    allow_transforms: bool,
    overload: i32,
) -> EncodeResult {
    let mut buf = vec![FILL; claimed + SLACK];
    let mut result_length = claimed;
    let ok = unsafe {
        (api.encode)(
            font.as_ptr(),
            font.len(),
            buf.as_mut_ptr(),
            &mut result_length,
            meta.as_ptr(),
            meta.len(),
            quality,
            allow_transforms as c_int,
            overload,
        )
    } != 0;
    EncodeResult {
        ok,
        result_length,
        buf,
    }
}

fn diff_encode_one(
    font: &[u8],
    claimed: usize,
    meta: &[u8],
    quality: i32,
    allow_transforms: bool,
    overload: i32,
) -> Option<Vec<u8>> {
    let r = encode(
        &REF,
        font,
        claimed,
        meta,
        quality,
        allow_transforms,
        overload,
    );
    let n = encode(
        &NEW,
        font,
        claimed,
        meta,
        quality,
        allow_transforms,
        overload,
    );
    let what = format!(
        "ConvertTTFToWOFF2(result_length={claimed} quality={quality} allow_transforms={allow_transforms} meta={} overload={overload})",
        meta.len()
    );
    // Bytes past the claimed length: the reference must not write there (that
    // would be a buffer overflow in the original), the new one never does.
    let ref_overflow = r.buf[claimed..].iter().any(|&b| b != FILL);
    if n.buf[claimed..].iter().any(|&b| b != FILL) {
        panic!("{what}: the Rust implementation wrote past the end of the result buffer");
    }
    if ref_overflow {
        // Undefined behavior in the original; it then always fails its final
        // length check. The new implementation must fail too.
        assert!(!r.ok, "{what}: reference overflowed but succeeded");
        check_eq!(format!("{what} result (reference overflowed)"), r.ok, n.ok);
        check_eq!(
            format!("{what} result_length"),
            r.result_length,
            n.result_length
        );
        check_bytes(
            &format!("{what} output"),
            &r.buf[..claimed],
            &n.buf[..claimed],
        );
        return None;
    }
    check_eq!(format!("{what} result"), r.ok, n.ok);
    check_eq!(
        format!("{what} result_length"),
        r.result_length,
        n.result_length
    );
    check_bytes(&format!("{what} output"), &r.buf, &n.buf);
    if r.ok {
        Some(r.buf[..r.result_length].to_vec())
    } else {
        None
    }
}

/// Fuzzer input layout for encoding: [quality, flags, meta_len, size_mode,
/// meta..., font...].
pub fn diff_encode(input: &[u8]) {
    if input.len() < 4 {
        return;
    }
    let (q, flags, meta_len, size_mode) = (input[0], input[1], input[2], input[3]);
    let rest = &input[4..];
    let meta_len = usize::from(meta_len).min(rest.len());
    let (meta, font) = rest.split_at(meta_len);
    let meta = if flags & 4 != 0 { meta } else { &meta[..0] };

    let overload = if flags & 2 != 0
        && (font.len() < 8192 || std::env::var_os("WOFF2_FUZZ_NO_QUALITY_CAP").is_some())
    {
        0
    } else {
        1
    };
    let mut quality = i32::from(q % 12);
    if font.len() > 32 * 1024 && std::env::var_os("WOFF2_FUZZ_NO_QUALITY_CAP").is_none() {
        // Keep fuzzing fast; brotli itself is the same library on both sides.
        quality = quality.min(5);
    }
    let allow_transforms = flags & 1 != 0;

    for ov in 0..2 {
        let r = unsafe {
            (REF.max_compressed_size)(font.as_ptr(), font.len(), meta.as_ptr(), meta.len(), ov)
        };
        let n = unsafe {
            (NEW.max_compressed_size)(font.as_ptr(), font.len(), meta.as_ptr(), meta.len(), ov)
        };
        check_eq!(format!("MaxWOFF2CompressedSize overload {ov}"), r, n);
    }
    let max = unsafe {
        (REF.max_compressed_size)(font.as_ptr(), font.len(), meta.as_ptr(), meta.len(), 1)
    };

    let claimed = match size_mode % 4 {
        0 | 3 => max,
        1 => max.saturating_sub(usize::from(size_mode) * 5),
        _ => usize::from(size_mode) * 3,
    };
    let out = diff_encode_one(font, claimed, meta, quality, allow_transforms, overload);

    if let Some(woff2) = out {
        if size_mode % 4 == 3 {
            // Exactly sized and one byte too small.
            diff_encode_one(font, woff2.len(), meta, quality, allow_transforms, overload);
            diff_encode_one(
                font,
                woff2.len() - 1,
                meta,
                quality,
                allow_transforms,
                overload,
            );
        }
        // Decoding what we produced exercises the decoder with valid input.
        diff_decode_string(&woff2, 0, None);
    }
}

// ---------------------------------------------------------------------------
// libwoff2common
// ---------------------------------------------------------------------------

/// Compares the libwoff2common helper functions.
pub fn diff_common(data: &[u8]) {
    let tags_r = unsafe { std::slice::from_raw_parts((REF.known_tags)(), 63) };
    let tags_n = unsafe { std::slice::from_raw_parts((NEW.known_tags)(), 63) };
    check_eq!("kKnownTags", tags_r, tags_n);
    check_eq!(
        "kKnownTags vs Rust",
        tags_r,
        &woff2::table_tags::KNOWN_TAGS[..]
    );

    let u32_at = |i: usize| -> u32 {
        let mut b = [0u8; 4];
        for (k, x) in b.iter_mut().enumerate() {
            *x = data.get(i + k).copied().unwrap_or(0);
        }
        u32::from_le_bytes(b)
    };
    let u64_at = |i: usize| -> u64 { u64::from(u32_at(i)) << 32 | u64::from(u32_at(i + 4)) };

    unsafe {
        check_eq!(
            "ComputeULongSum",
            (REF.ulong_sum)(data.as_ptr(), data.len()),
            (NEW.ulong_sum)(data.as_ptr(), data.len())
        );
        let (v, n) = (u32_at(0), u32_at(4));
        for (v, n) in [(v, n), (0x0001_0000, n), (0x0002_0000, n), (v, n & 0xffff)] {
            check_eq!(
                format!("CollectionHeaderSize({v:#x}, {n})"),
                (REF.collection_header_size)(v, n),
                (NEW.collection_header_size)(v, n)
            );
        }
        let x = u32_at(8);
        check_eq!(
            "CompressedBufferSize",
            (REF.compressed_buffer_size)(x),
            (NEW.compressed_buffer_size)(x)
        );

        for v in [x as i32, (x & 0xffff) as i32, (x & 0x3ff) as i32] {
            let (mut a, mut b) = ([0u8; 8], [0u8; 8]);
            let la = (REF.write_255_ushort)(v, a.as_mut_ptr());
            let lb = (NEW.write_255_ushort)(v, b.as_mut_ptr());
            check_eq!(format!("Write255UShort({v})"), (la, a), (lb, b));
            let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
            let start = (x >> 28) as usize;
            let la = (REF.store_255_ushort)(v, start, a.as_mut_ptr());
            let lb = (NEW.store_255_ushort)(v, start, b.as_mut_ptr());
            check_eq!(format!("Store255UShort({v})"), (la, a), (lb, b));
            check_eq!(
                format!("Size255UShort({v})"),
                (REF.size_255_ushort)(v as u16),
                (NEW.size_255_ushort)(v as u16)
            );
        }

        let y = u64_at(12) as usize;
        for v in [y, y & 0xffff_ffff, y & 0xfffff, y & 0x7f] {
            check_eq!(
                format!("Base128Size({v})"),
                (REF.base128_size)(v),
                (NEW.base128_size)(v)
            );
            let (mut a, mut b) = ([0u8; 32], [0u8; 32]);
            let start = (x >> 28) as usize;
            let la = (REF.store_base128)(v, start, a.as_mut_ptr());
            let lb = (NEW.store_base128)(v, start, b.as_mut_ptr());
            check_eq!(format!("StoreBase128({v})"), (la, a), (lb, b));
        }

        for start in 0..data.len().min(24) + 1 {
            let (mut oa, mut ob, mut va, mut vb) = (0usize, 0usize, 0u32, 0u32);
            let ra = (REF.read_255_ushort)(data.as_ptr(), data.len(), start, &mut oa, &mut va);
            let rb = (NEW.read_255_ushort)(data.as_ptr(), data.len(), start, &mut ob, &mut vb);
            check_eq!(
                format!("Read255UShort at {start}"),
                (ra, oa, va),
                (rb, ob, vb)
            );
            let ra = (REF.read_base128)(data.as_ptr(), data.len(), start, &mut oa, &mut va);
            let rb = (NEW.read_base128)(data.as_ptr(), data.len(), start, &mut ob, &mut vb);
            check_eq!(
                format!("ReadBase128 at {start}"),
                (ra, oa, va),
                (rb, ob, vb)
            );
        }
    }
}
