# Differential fuzzing

The fuzzers link two complete implementations into one binary:

* the original C++ implementation (`reference/`, renamed to namespace
  `woff2_ref`), and
* the new implementation, consisting of the C++ ABI layer (`../cpp`) and the
  Rust crate.

`glue/glue.cc` is compiled once against each of them, so both are driven
through exactly the public C++ API that applications use. `src/lib.rs` then
compares everything observable:

* return values and out-parameters (`ComputeWOFF2FinalSize`,
  `MaxWOFF2CompressedSize` (both overloads), `*result_length`, ...);
* all output bytes, including caller buffer bytes that must *not* be
  written (buffers are pre-filled with a pattern, with slack after the end);
* decoding into `WOFF2StringOut` (empty, pre-sized, with various
  `SetMaxSize` limits), into the deprecated fixed buffer API
  (`WOFF2MemoryOut`), and into a user-defined `WOFF2Out`. For the latter, the
  complete sequence of virtual calls (`Write`, positioned `Write`, `Size`)
  with offsets, lengths and data hashes must be identical. This is checked
  again with a failing `Write` and with an exception thrown at various call
  numbers;
* encoding with both overloads, all brotli qualities, with and without
  transforms and extended metadata, and with result buffers that are too
  small, exactly sized or one byte too small. Successful outputs are decoded
  again by both implementations;
* the `libwoff2common` helpers (`Read255UShort`, `ReadBase128`,
  `Write255UShort`, `Store255UShort`, `StoreBase128`, `ComputeULongSum`,
  `CollectionHeaderSize`, `kKnownTags`, ...).

Any difference panics with a description of the first mismatch.

## Targets

| target              | input                                                         |
|---------------------|---------------------------------------------------------------|
| `decode`            | a WOFF2 file                                                  |
| `decode_structured` | a WOFF2 file whose compressed block is stored *uncompressed*; the harness compresses it and fixes up the header, so that mutations reach the font reconstruction. The header's `reserved` field carries knobs for the fix-ups. |
| `encode`            | `[quality, flags, meta_len, size_mode, metadata..., font...]` |
| `common`            | arbitrary bytes                                               |

## Running

Requires nightly Rust and `cargo install cargo-fuzz`. Both implementations are
built with AddressSanitizer. On macOS, Apple clang's ASan instrumentation is
incompatible with Rust's ASan runtime. The build therefore uses Homebrew's
LLVM clang (`brew install llvm`), or `WOFF2_FUZZ_CXX=/path/to/clang++`.

```
# Seed corpora: synthetic fonts, plus (optionally) minified local fonts.
(cd seedgen && cargo build --release)
./seedgen/target/release/woff2-seedgen corpus /System/Library/Fonts

cargo +nightly fuzz run decode            corpus/decode            -- -max_len=65536
cargo +nightly fuzz run decode_structured corpus/decode_structured -- -max_len=65536
cargo +nightly fuzz run encode            corpus/encode            -- -max_len=65536
cargo +nightly fuzz run common            corpus/common
```

`woff2-seedgen --regression corpus <font dirs>` also writes the given fonts
unmodified (`corpus/regression_{encode,decode}`), as a full-size regression
test:

```
WOFF2_FUZZ_NO_QUALITY_CAP=1 cargo +nightly fuzz run encode corpus/regression_encode -- -runs=0 -max_len=100000000
cargo +nightly fuzz run decode corpus/regression_decode -- -runs=0 -max_len=100000000
```

Font data is not checked in, because system fonts are generally not
redistributable.
