//! WOFF2 font compression and decompression.
//!
//! This crate is a Rust port of the reference [google/woff2] library. It is
//! designed as a drop-in replacement: the C++ layer in `cpp/` builds
//! `libwoff2common`, `libwoff2dec` and `libwoff2enc` with exactly the same
//! public C++ API and ABI as the original library, forwarding all of the
//! actual work to this crate.
//!
//! The port is intentionally *bit-for-bit* faithful, including the exact
//! sequence of calls made on a [`Woff2Out`] while decoding, and has been
//! validated against the original implementation with differential fuzzing
//! (see `fuzz/`).
//!
//! [google/woff2]: https://github.com/google/woff2

#![deny(unsafe_op_in_unsafe_fn)]

#[macro_use]
mod log;

pub mod brotli;
pub mod buffer;
pub mod common;
pub mod decode;
pub mod encode;
pub mod output;
pub mod store;
pub mod table_tags;
pub mod variable_length;

#[cfg(feature = "capi")]
pub mod capi;

pub use decode::{compute_woff2_final_size, convert_woff2_to_ttf_with};
pub use encode::{convert_ttf_to_woff2_with, max_woff2_compressed_size, Woff2Params};
pub use output::{MemoryOut, StringOut, Woff2Out, DEFAULT_MAX_SIZE};

#[cfg(feature = "system-brotli")]
pub use decode::convert_woff2_to_ttf;
#[cfg(feature = "system-brotli")]
pub use encode::convert_ttf_to_woff2;
