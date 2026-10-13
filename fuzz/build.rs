//! Builds, into the fuzzer binaries:
//!  * the original C++ implementation (the fuzz/reference submodule, with
//!    reference-ub-fixes.patch applied to a copy), in namespace `woff2_ref`,
//!  * the new C++ ABI layer (cpp/), which forwards to the Rust crate,
//!  * the C glue (fuzz/glue/glue.cc) for both.
//!
//! When the fuzzers are built with a sanitizer (cargo fuzz default: ASan),
//! the C++ code is instrumented as well, including coverage feedback.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Copies the reference sources (the fuzz/reference submodule, google/woff2)
/// into OUT_DIR and applies reference-ub-fixes.patch there, leaving the
/// submodule itself untouched. Returns the patched `src` directory.
fn patched_reference(fuzz: &Path) -> PathBuf {
    let src = fuzz.join("reference").join("src");
    if !src.join("woff2_dec.cc").exists() {
        panic!(
            "\n\nThe reference implementation (google/woff2) is not checked out. \
             Fetch it with:\n\n    git submodule update --init --checkout fuzz/reference\n\n"
        );
    }
    let out = PathBuf::from(env::var("OUT_DIR").unwrap()).join("reference");
    let _ = fs::remove_dir_all(&out);
    fs::create_dir_all(out.join("src")).unwrap();
    for entry in fs::read_dir(&src).unwrap() {
        let path = entry.unwrap().path();
        if matches!(path.extension().and_then(|e| e.to_str()), Some("cc" | "h")) {
            fs::copy(&path, out.join("src").join(path.file_name().unwrap())).unwrap();
        }
    }
    let status = Command::new("patch")
        .args(["-p1", "-N", "-s", "-d"])
        .arg(&out)
        .arg("-i")
        .arg(fuzz.join("reference-ub-fixes.patch"))
        .status()
        .expect("failed to run `patch`");
    assert!(status.success(), "applying reference-ub-fixes.patch failed");
    out.join("src")
}

fn brotli_include() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Ok(out) = Command::new("pkg-config")
        .args(["--cflags-only-I", "libbrotlienc", "libbrotlidec"])
        .output()
    {
        for flag in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            if let Some(d) = flag.strip_prefix("-I") {
                dirs.push(PathBuf::from(d));
            }
        }
    }
    dirs
}

/// On macOS, Apple clang's ASan instrumentation is incompatible with the ASan
/// runtime shipped with rustc. Use an upstream clang (`WOFF2_FUZZ_CXX`, or
/// Homebrew's LLVM) with the SDK's libc++ headers instead.
fn configure_compiler(b: &mut cc::Build, sanitize: bool) {
    let target = env::var("TARGET").unwrap_or_default();
    if !target.contains("apple") {
        return;
    }
    b.flag("-mmacosx-version-min=11.0");
    let cxx = env::var("WOFF2_FUZZ_CXX")
        .ok()
        .map(PathBuf::from)
        .or_else(|| {
            let brew = PathBuf::from("/opt/homebrew/opt/llvm/bin/clang++");
            brew.exists().then_some(brew)
        });
    if !sanitize {
        return;
    }
    let cxx =
        cxx.expect("set WOFF2_FUZZ_CXX to an upstream (non-Apple) clang++ to build with ASan");
    let sdk = Command::new("xcrun")
        .arg("--show-sdk-path")
        .output()
        .expect("xcrun");
    let sdk = String::from_utf8_lossy(&sdk.stdout).trim().to_string();
    b.compiler(cxx)
        .flag("-isysroot")
        .flag(&sdk)
        .flag("-nostdinc++")
        .flag("-isystem")
        .flag(format!("{sdk}/usr/include/c++/v1"));
}

fn base() -> cc::Build {
    let mut b = cc::Build::new();
    b.cpp(true)
        .std("c++11")
        .warnings(false)
        .define("__STDC_FORMAT_MACROS", None)
        .flag_if_supported("-fno-omit-frame-pointer")
        .opt_level(1)
        .debug(true);
    for d in brotli_include() {
        b.include(d);
    }
    let sanitizers = env::var("CARGO_CFG_SANITIZE").unwrap_or_default();
    let asan = sanitizers.split(',').any(|s| s == "address");
    configure_compiler(&mut b, asan);
    if asan {
        b.flag("-fsanitize=address");
    }
    if env::var("CARGO_CFG_FUZZING").is_ok() {
        b.flag_if_supported("-fsanitize-coverage=inline-8bit-counters,pc-table,trace-cmp");
    }
    b
}

fn main() {
    let fuzz = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let root = fuzz.parent().unwrap().to_path_buf();
    let reference = fuzz.join("reference");
    let reference_src = patched_reference(&fuzz);

    println!("cargo:rerun-if-changed=glue/glue.cc");
    println!("cargo:rerun-if-changed=reference/src");
    println!("cargo:rerun-if-changed=reference/include");
    println!("cargo:rerun-if-changed=reference-ub-fixes.patch");
    println!("cargo:rerun-if-changed=../cpp");
    println!("cargo:rerun-if-changed=../include");
    println!("cargo:rerun-if-env-changed=WOFF2_FUZZ_CXX");

    // Original implementation, renamed to namespace woff2_ref.
    let ref_sources = [
        "font.cc",
        "glyph.cc",
        "normalize.cc",
        "table_tags.cc",
        "transform.cc",
        "variable_length.cc",
        "woff2_common.cc",
        "woff2_dec.cc",
        "woff2_enc.cc",
        "woff2_out.cc",
    ];
    let mut b = base();
    b.define("woff2", "woff2_ref")
        .include(reference.join("include"))
        .include(&reference_src);
    for s in ref_sources {
        b.file(reference_src.join(s));
    }
    b.define("GLUE_REF", None)
        .define("GLUE_PREFIX", "ref_")
        .file(fuzz.join("glue/glue.cc"));
    b.compile("woff2_reference");

    // New implementation: C++ ABI layer + glue (the Rust code comes from the
    // woff2 crate dependency).
    let mut b = base();
    b.include(root.join("include")).include(root.join("cpp"));
    for s in ["common.cc", "decode.cc", "encode.cc", "output.cc"] {
        b.file(root.join("cpp").join(s));
    }
    b.define("GLUE_PREFIX", "new_")
        .file(fuzz.join("glue/glue.cc"));
    b.compile("woff2_new_cxx");
}
