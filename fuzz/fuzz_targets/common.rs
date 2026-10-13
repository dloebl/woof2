#![no_main]
//! Differential fuzzing of the libwoff2common helper functions.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    woff2_fuzz::diff_common(data);
});
