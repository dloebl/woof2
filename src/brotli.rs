//! Minimal brotli interface.
//!
//! The WOFF2 code needs exactly two brotli entry points: one-shot
//! decompression and one-shot compression. To keep the dependency structure
//! of the original library (only `libwoff2dec` depends on `libbrotlidec` and
//! only `libwoff2enc` depends on `libbrotlienc`), the core takes these as
//! plain C function pointers with the signatures of `BrotliDecoderDecompress`
//! and `BrotliEncoderCompress`. With the `system-brotli` feature the crate
//! links the system brotli libraries and provides them directly.

use std::os::raw::c_int;

/// Signature of `BrotliDecoderDecompress` (the `BrotliDecoderResult` return
/// value is an `int`-sized enum).
pub type DecompressFn = unsafe extern "C" fn(
    encoded_size: usize,
    encoded_buffer: *const u8,
    decoded_size: *mut usize,
    decoded_buffer: *mut u8,
) -> c_int;

/// Signature of `BrotliEncoderCompress` (`mode` is a `BrotliEncoderMode`,
/// the return value a `BROTLI_BOOL`).
pub type CompressFn = unsafe extern "C" fn(
    quality: c_int,
    lgwin: c_int,
    mode: c_int,
    input_size: usize,
    input_buffer: *const u8,
    encoded_size: *mut usize,
    encoded_buffer: *mut u8,
) -> c_int;

pub const BROTLI_DECODER_RESULT_SUCCESS: c_int = 1;
pub const BROTLI_MODE_TEXT: c_int = 1;
pub const BROTLI_MODE_FONT: c_int = 2;
pub const BROTLI_DEFAULT_WINDOW: c_int = 22;

/// Decompresses `src` into exactly `dst.len()` bytes. Returns `true` only if
/// the stream decoded successfully to exactly that size.
pub(crate) fn decompress_exact(decompress: DecompressFn, src: &[u8], dst: &mut [u8]) -> bool {
    let mut decoded_size = dst.len();
    // SAFETY: both pointers are valid for the given sizes for the duration
    // of the call, which is the contract of `BrotliDecoderDecompress`.
    let result =
        unsafe { decompress(src.len(), src.as_ptr(), &mut decoded_size, dst.as_mut_ptr()) };
    result == BROTLI_DECODER_RESULT_SUCCESS && decoded_size == dst.len()
}

/// Compresses `input` into `output`, returning the compressed size.
pub(crate) fn compress(
    compress: CompressFn,
    quality: i32,
    mode: c_int,
    input: &[u8],
    output: &mut [u8],
) -> Option<usize> {
    let mut encoded_size = output.len();
    // SAFETY: both pointers are valid for the given sizes for the duration
    // of the call, which is the contract of `BrotliEncoderCompress`.
    let ok = unsafe {
        compress(
            quality,
            BROTLI_DEFAULT_WINDOW,
            mode,
            input.len(),
            input.as_ptr(),
            &mut encoded_size,
            output.as_mut_ptr(),
        )
    };
    if ok == 0 {
        None
    } else {
        Some(encoded_size)
    }
}

#[cfg(feature = "system-brotli")]
mod system {
    use std::os::raw::c_int;

    extern "C" {
        pub fn BrotliDecoderDecompress(
            encoded_size: usize,
            encoded_buffer: *const u8,
            decoded_size: *mut usize,
            decoded_buffer: *mut u8,
        ) -> c_int;

        pub fn BrotliEncoderCompress(
            quality: c_int,
            lgwin: c_int,
            mode: c_int,
            input_size: usize,
            input_buffer: *const u8,
            encoded_size: *mut usize,
            encoded_buffer: *mut u8,
        ) -> c_int;
    }
}

/// `BrotliDecoderDecompress` from the system `libbrotlidec`.
#[cfg(feature = "system-brotli")]
pub const SYSTEM_DECOMPRESS: DecompressFn = system::BrotliDecoderDecompress;

/// `BrotliEncoderCompress` from the system `libbrotlienc`.
#[cfg(feature = "system-brotli")]
pub const SYSTEM_COMPRESS: CompressFn = system::BrotliEncoderCompress;
