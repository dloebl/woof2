# woof2

WOFF2 font compression in Rust, a drop-in replacement for
[google/woff2](https://github.com/google/woff2). Same C++ API and ABI,
byte-identical output, verified by differential fuzzing.

brotli/ contains reference code for the Brotli byte-level compression
algorithm. Note that it is licensed under the MIT license.

src/ contains the Rust implementation of compressing and decompressing fonts
(WOFF 2.0). cpp/ contains a thin C++ layer that provides the original C++ API
and ABI on top of it.

# Rust implementation, C++ API and ABI

The library is implemented in Rust. It is a **drop-in replacement** for the
original C++ implementation: existing users do not need to change anything.

* **API**: the installed headers `woff2/decode.h`, `woff2/encode.h` and
  `woff2/output.h` are unchanged.
* **ABI**: the build produces the same libraries (`libwoff2common`,
  `libwoff2dec`, `libwoff2enc`) with the same names, SONAMEs and versions
  (1.0.2), the same pkg-config files and the same dependency structure
  (`libwoff2dec` → `libbrotlidec`, `libwoff2enc` → `libbrotlienc`, both →
  `libwoff2common`). Every symbol declared in the installed headers is
  exported with the same mangled name, including the vtables and typeinfo of
  `WOFF2StringOut` and `WOFF2MemoryOut`, as are the helper functions exported
  by `libwoff2common` (`ReadBase128`, `Read255UShort`, `kKnownTags`, ...).
  Applications linked against the original libraries run unchanged against
  the new ones.
* **Behavior**: the output is byte-for-byte identical. This includes encoded
  and decoded fonts, return values, output size parameters, the bytes left
  untouched in caller buffers, and the exact sequence of calls made on a
  user-defined `WOFF2Out`. Exceptions thrown by a `WOFF2Out` implementation
  propagate to the caller as before. The diagnostics printed with
  `NOISY_LOGGING` are identical too. This was verified with differential
  fuzzing against the original implementation (see `fuzz/README.md`).

The only intentional differences are:

* Inputs on which the original code had undefined behavior (heap buffer
  overflows, see `fuzz/README.md`) are rejected cleanly (the
  conversion returns `false`).
* If one of the large buffers (sized from the input) cannot be allocated, the
  conversion returns `false` instead of throwing `std::bad_alloc`. Rust
  panics never unwind into C++: should one ever occur, the conversion returns
  `false`.
* `libwoff2enc` no longer exports its private internals (`woff2::Font`,
  `woff2::Glyph`, `woff2::NormalizeFont`, ...), which were never declared in
  an installed header. Only the public API (plus `CompressedBufferSize`) is
  exported. `libwoff2common` additionally exports `woff2_rs_*` C functions
  that `libwoff2dec` and `libwoff2enc` use internally. None of the Rust
  standard library's symbols are exported.

The architecture is:

```
include/woff2/*.h       unchanged public C++ API
cpp/                    C++ ABI layer: WOFF2Out adapters, exception
                        forwarding, WOFF2StringOut/WOFF2MemoryOut, the
                        libwoff2common helpers, export lists
src/                    the Rust crate: decoder, encoder (font parsing,
                        normalization, glyf/loca transform), command line
                        tools
fuzz/                   differential fuzzing against the original C++ code
                        (google/woff2 as the fuzz/reference submodule)
```

The Rust code is linked into `libwoff2common`. `libwoff2dec` and
`libwoff2enc` pass the brotli entry points into it. This keeps `libwoff2common`
free of brotli dependencies, as before.

# Build & Run

The build requires a C++11 compiler and a Rust toolchain (rustc 1.63 or newer
and cargo 1.64 or newer, see https://rustup.rs).

## Build

On a standard Unix-style environment:

```
git clone --recursive https://github.com/dloebl/woof2.git
cd woof2
make clean all
```

Alternatively, if Brotli is already installed on your system you can use CMake
to build executables and libraries:

```
git clone https://github.com/dloebl/woof2.git
cd woof2
mkdir out
cd out
cmake ..
make
make install
```

By default, shared libraries are built. To use static linkage, do:

```
cd woof2
mkdir out-static
cd out-static
cmake -DBUILD_SHARED_LIBS=OFF ..
make
make install
```

The static `libwoff2common.a` contains the Rust implementation. Use
`pkg-config --static --libs libwoff2dec libwoff2enc` to get the complete set
of libraries to link.

CMake finds `cargo` in `PATH` or `~/.cargo/bin`, or you can pass
`-DCARGO=/path/to/cargo`. `-DWOFF2_CARGO_TARGET=<triple>` cross compiles the
Rust part.

## Using the Rust crate directly

The crate can also be used from Rust (`woff2` in `Cargo.toml`). The default
`system-brotli` feature links the system brotli libraries:

```rust
let woff2_data = woff2::convert_ttf_to_woff2(&ttf, &woff2::Woff2Params::default())
    .expect("invalid font");
let mut ttf_again = Vec::new();
let mut out = woff2::StringOut::new(&mut ttf_again);
assert!(woff2::convert_woff2_to_ttf(&woff2_data, &mut out));
```

## Run

Ensure the binaries from the build process are in your $PATH, then:

```
woff2_compress myfont.ttf
woff2_decompress myfont.woff2
```

## Test

```
cargo test                               # Rust unit and round trip tests
cd fuzz && cargo +nightly fuzz run ...   # differential fuzzing, see fuzz/README.md
```

`tests/abi/abi_test.cc` is an ABI test. Build it against the original C++
library, then run the same binary against the new libraries
(`LD_LIBRARY_PATH`/`DYLD_LIBRARY_PATH`). Its output must be identical.

The Makefile still produces the self-contained `src/convert_woff2ttf_fuzzer.a`
and `src/convert_woff2ttf_fuzzer_new_entry.a` archives used by OSS-Fuzz. They
now include the Rust code, so the build image needs a Rust toolchain (e.g.
OSS-Fuzz's `base-builder-rust`). Set `RUSTFLAGS` to instrument the Rust code
as well.

# References

http://www.w3.org/TR/WOFF2/
http://www.w3.org/Submission/MTX/
