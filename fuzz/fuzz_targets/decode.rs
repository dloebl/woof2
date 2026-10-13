#![no_main]
//! Differential fuzzing of WOFF2 decoding (raw WOFF2 input).
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    woff2_fuzz::diff_decode(data);
});
