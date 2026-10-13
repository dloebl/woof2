#![no_main]
//! Differential fuzzing of TTF/OTF/TTC -> WOFF2 encoding.
//! Input: [quality, flags, meta_len, size_mode, meta..., font...]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    woff2_fuzz::diff_encode(data);
});
