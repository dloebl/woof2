# Reference implementation

This directory contains the original C++ implementation of woff2, as of
upstream commit `fb9c337` ("Make sure endPtsOfContours is monotonic."). It is
used as the oracle for differential fuzzing. It is not part of the library
build.

When the fuzzers are built, it is compiled with `-Dwoff2=woff2_ref`, so that it
can be linked into the same binary as the new implementation.

## Patches

The reference code has a few memory-safety bugs (undefined behavior). Under
AddressSanitizer they would abort the fuzzer, and without ASan the resulting
memory corruption would make comparisons meaningless. Each bug is patched so
that the reference fails cleanly at that point, which is what the Rust
implementation does. The patches are marked with `FUZZ-REFERENCE-PATCH(n)`:

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
