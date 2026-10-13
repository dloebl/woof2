# Differential fuzzing

The fuzzers link two complete implementations into one binary:

* the original C++ implementation (the `reference/` submodule, see below,
  renamed to namespace `woff2_ref`), and
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

## Reference implementation

`reference/` is a git submodule with the original C++ implementation,
google/woff2 at commit `fb9c337` ("Make sure endPtsOfContours is monotonic."),
the version this port matches. It is only needed for fuzzing, so it is marked
`update = none` and has to be fetched explicitly (see above). `build.rs`
compiles it with `-Dwoff2=woff2_ref`, so that it can be linked into the same
binary as the new implementation.

### Patches

The reference code has a few memory-safety bugs (undefined behavior). Under
AddressSanitizer they would abort the fuzzer, and without ASan the resulting
memory corruption would make comparisons meaningless. Each bug is patched so
that the reference fails cleanly at that point, which is what the Rust
implementation does. The patches are in `reference-ub-fixes.patch`; `build.rs`
applies them to a copy of the sources in its output directory, so the
submodule stays pristine. They are marked with `FUZZ-REFERENCE-PATCH(n)`:

1. `normalize.cc`, `WriteNormalizedLoca`: rounding the size of a normalized
   glyph up to a multiple of 4 can move `glyf_offset` past the end of the
   output buffer. The next `buffer.size() - glyf_offset` then wraps around,
   and `StoreGlyph()` writes out of bounds. The patch saturates the available
   size at 0. That matches the original wherever its behavior is defined: an
   empty glyph still succeeds, and anything else fails the size check.
2. `normalize.cc`, `NormalizeGlyphs`: when a short `loca` has to be rewritten
   as a long one and the font's `head` table is shared with an earlier font
   of a collection, `head_table->buffer` is empty and `buffer[51] = 1` writes
   out of bounds. The patch makes the conversion fail.
3. `woff2_enc.cc`, `ConvertTTFToWOFF2`: in some malformed collections, a
   table that is "reused" from another font is copied into `transform_buf`
   although it was not counted in `total_transform_length`. This overflows the
   buffer. The patch makes the conversion fail. It also uses
   `transform_buf.data()` instead of `&transform_buf[0]`, which is undefined
   for an empty vector.

Writes past the end of the caller's result buffer in `ConvertTTFToWOFF2`
(when the precomputed length is smaller than the data actually written) are
not patched. Instead, the harness gives the reference extra slack. If the
reference writes into the slack, its conversion always fails its final length
check, and the harness requires the new implementation to fail as well. The
new implementation never writes past `*result_length`.

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
# The reference implementation (not fetched by `git clone --recursive`).
git submodule update --init --checkout fuzz/reference

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
