#![no_main]
//! Differential fuzzing of WOFF2 decoding. The input is a WOFF2 file whose
//! compressed data block is stored *uncompressed*; the harness compresses it
//! and fixes up the header, so that mutations reach the font reconstruction.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if let Some(woff2) = woff2_fuzz::structured_to_woff2(data) {
        woff2_fuzz::diff_decode(&woff2);
    }
});
