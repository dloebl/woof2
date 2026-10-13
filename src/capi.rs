//! C entry points used by the C++ ABI layer in `cpp/`.
//!
//! These are an implementation detail of `libwoff2common`, `libwoff2dec` and
//! `libwoff2enc`; the public interface is the unchanged C++ API declared in
//! `include/woff2/*.h`. See `cpp/woff2_rs.h` for the matching declarations.
//!
//! No Rust panic ever crosses this boundary: every entry point catches
//! unwinding and reports failure instead.

use std::mem::MaybeUninit;
use std::os::raw::{c_int, c_uint, c_void};
use std::panic::{catch_unwind, AssertUnwindSafe};

use crate::brotli::{CompressFn, DecompressFn};
use crate::buffer::Buffer;
use crate::output::Woff2Out;
use crate::{common, decode, encode, variable_length};

/// Callbacks forwarding to a C++ `woff2::WOFF2Out`.
#[repr(C)]
pub struct Woff2RsOut {
    pub ctx: *mut c_void,
    pub write: unsafe extern "C" fn(ctx: *mut c_void, buf: *const u8, n: usize) -> bool,
    pub write_at:
        unsafe extern "C" fn(ctx: *mut c_void, buf: *const u8, offset: usize, n: usize) -> bool,
    pub size: unsafe extern "C" fn(ctx: *mut c_void) -> usize,
}

struct CallbackOut<'a>(&'a Woff2RsOut);

impl Woff2Out for CallbackOut<'_> {
    fn write(&mut self, buf: &[u8]) -> bool {
        // SAFETY: the C++ layer guarantees a valid context and callbacks.
        unsafe { (self.0.write)(self.0.ctx, buf.as_ptr(), buf.len()) }
    }

    fn write_at(&mut self, buf: &[u8], offset: usize) -> bool {
        // SAFETY: the C++ layer guarantees a valid context and callbacks.
        unsafe { (self.0.write_at)(self.0.ctx, buf.as_ptr(), offset, buf.len()) }
    }

    fn size(&mut self) -> usize {
        // SAFETY: the C++ layer guarantees a valid context and callbacks.
        unsafe { (self.0.size)(self.0.ctx) }
    }
}

/// Builds a slice from a C pointer/length pair, accepting NULL for empty.
unsafe fn slice<'a>(data: *const u8, length: usize) -> &'a [u8] {
    if data.is_null() || length == 0 {
        &[]
    } else {
        // SAFETY: guaranteed by the caller (the C++ API contract).
        unsafe { std::slice::from_raw_parts(data, length) }
    }
}

fn guard<T>(default: T, f: impl FnOnce() -> T) -> T {
    catch_unwind(AssertUnwindSafe(f)).unwrap_or(default)
}

// ---------------------------------------------------------------------------
// libwoff2dec
// ---------------------------------------------------------------------------

/// `woff2::ComputeWOFF2FinalSize`.
///
/// # Safety
/// `data` must be valid for reads of `length` bytes (or NULL if 0).
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_compute_final_size(data: *const u8, length: usize) -> usize {
    // SAFETY: forwarded caller guarantee.
    let data = unsafe { slice(data, length) };
    guard(0, || decode::compute_woff2_final_size(data))
}

/// `woff2::ConvertWOFF2ToTTF(const uint8_t*, size_t, WOFF2Out*)`.
///
/// # Safety
/// `data` must be valid for reads of `length` bytes (or NULL if 0), `out`
/// must point to valid callbacks, and `decompress` must behave like
/// `BrotliDecoderDecompress`.
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_convert_woff2_to_ttf(
    data: *const u8,
    length: usize,
    out: *const Woff2RsOut,
    decompress: DecompressFn,
) -> bool {
    // SAFETY: forwarded caller guarantees.
    let (data, out) = unsafe { (slice(data, length), &*out) };
    guard(false, || {
        let mut out = CallbackOut(out);
        decode::convert_woff2_to_ttf_with(data, &mut out, decompress)
    })
}

// ---------------------------------------------------------------------------
// libwoff2enc
// ---------------------------------------------------------------------------

/// `woff2::MaxWOFF2CompressedSize`.
///
/// # Safety
/// `data` must be valid for reads of `length` bytes (or NULL if 0).
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_max_compressed_size(
    data: *const u8,
    length: usize,
    extended_metadata_length: usize,
) -> usize {
    let _ = data;
    encode::max_woff2_compressed_size(length, extended_metadata_length)
}

/// `woff2::CompressedBufferSize` (exported but undeclared in C++).
#[no_mangle]
pub extern "C" fn woff2_rs_compressed_buffer_size(original_size: u32) -> u32 {
    encode::compressed_buffer_size(original_size)
}

/// `woff2::ConvertTTFToWOFF2`.
///
/// # Safety
/// `data` must be valid for reads of `length` bytes, `result` for writes of
/// `*result_length` bytes (either may be NULL if the size is 0),
/// `extended_metadata` for reads of `extended_metadata_length` bytes, and
/// `compress` must behave like `BrotliEncoderCompress`.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn woff2_rs_convert_ttf_to_woff2(
    data: *const u8,
    length: usize,
    result: *mut u8,
    result_length: *mut usize,
    extended_metadata: *const u8,
    extended_metadata_length: usize,
    brotli_quality: c_int,
    allow_transforms: bool,
    compress: CompressFn,
) -> bool {
    // SAFETY: forwarded caller guarantees.
    let (data, metadata, result_length) = unsafe {
        (
            slice(data, length),
            slice(extended_metadata, extended_metadata_length),
            &mut *result_length,
        )
    };
    let result: &mut [MaybeUninit<u8>] = if result.is_null() || *result_length == 0 {
        &mut []
    } else {
        // SAFETY: the caller guarantees `*result_length` writable bytes.
        unsafe { std::slice::from_raw_parts_mut(result.cast(), *result_length) }
    };
    guard(false, || {
        let params = encode::Woff2Params {
            extended_metadata: metadata.to_vec(),
            brotli_quality,
            allow_transforms,
        };
        encode::convert_ttf_to_woff2_into(data, result, result_length, &params, compress)
    })
}

// ---------------------------------------------------------------------------
// libwoff2common
// ---------------------------------------------------------------------------

/// `woff2::ComputeULongSum`.
///
/// # Safety
/// `buf` must be valid for reads of `size` bytes (or NULL if 0).
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_compute_ulong_sum(buf: *const u8, size: usize) -> u32 {
    // SAFETY: forwarded caller guarantee.
    common::compute_ulong_sum(unsafe { slice(buf, size) })
}

/// `woff2::CollectionHeaderSize`.
#[no_mangle]
pub extern "C" fn woff2_rs_collection_header_size(header_version: u32, num_fonts: u32) -> usize {
    common::collection_header_size(header_version, num_fonts)
}

/// `woff2::Size255UShort`.
#[no_mangle]
pub extern "C" fn woff2_rs_size_255_ushort(value: u16) -> usize {
    variable_length::size_255_ushort(value)
}

/// Encodes `value` like `woff2::Write255UShort` into `out[0..3]`, returning
/// the number of bytes.
///
/// # Safety
/// `out` must be valid for writes of 3 bytes.
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_encode_255_ushort(value: c_int, out: *mut u8) -> usize {
    let mut tmp = [0u8; 3];
    let n = variable_length::encode_255_ushort(value, &mut tmp);
    // SAFETY: guaranteed by the caller.
    unsafe { std::ptr::copy_nonoverlapping(tmp.as_ptr(), out, n) };
    n
}

/// `woff2::Base128Size`.
#[no_mangle]
pub extern "C" fn woff2_rs_base128_size(n: usize) -> usize {
    variable_length::base128_size(n)
}

/// Encodes `len` like `woff2::StoreBase128` into `out[0..10]`, returning the
/// number of bytes.
///
/// # Safety
/// `out` must be valid for writes of 10 bytes.
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_encode_base128(len: usize, out: *mut u8) -> usize {
    let mut tmp = [0u8; 10];
    let n = variable_length::encode_base128(len, &mut tmp);
    // SAFETY: guaranteed by the caller.
    unsafe { std::ptr::copy_nonoverlapping(tmp.as_ptr(), out, n) };
    n
}

/// `woff2::Read255UShort` on the state of a `woff2::Buffer`. `*offset` is
/// updated even on failure, exactly like the C++ function.
///
/// # Safety
/// `data` must be valid for reads of `length` bytes (or NULL if 0), `offset`
/// and `value` must be valid pointers and `*offset <= length`.
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_read_255_ushort(
    data: *const u8,
    length: usize,
    offset: *mut usize,
    value: *mut c_uint,
) -> bool {
    // SAFETY: forwarded caller guarantees.
    let (data, offset, value) = unsafe { (slice(data, length), &mut *offset, &mut *value) };
    let mut buf = Buffer::with_offset(data, (*offset).min(data.len()));
    let result = variable_length::read_255_ushort(&mut buf);
    *offset = buf.offset();
    match result {
        Some(v) => {
            *value = v;
            true
        }
        None => false,
    }
}

/// `woff2::ReadBase128` on the state of a `woff2::Buffer`. `*offset` is
/// updated even on failure, exactly like the C++ function.
///
/// # Safety
/// `data` must be valid for reads of `length` bytes (or NULL if 0), `offset`
/// and `value` must be valid pointers and `*offset <= length`.
#[no_mangle]
pub unsafe extern "C" fn woff2_rs_read_base128(
    data: *const u8,
    length: usize,
    offset: *mut usize,
    value: *mut u32,
) -> bool {
    // SAFETY: forwarded caller guarantees.
    let (data, offset, value) = unsafe { (slice(data, length), &mut *offset, &mut *value) };
    let mut buf = Buffer::with_offset(data, (*offset).min(data.len()));
    let result = variable_length::read_base128(&mut buf);
    *offset = buf.offset();
    match result {
        Some(v) => {
            *value = v;
            true
        }
        None => false,
    }
}
