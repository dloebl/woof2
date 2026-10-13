//! Links libbrotlienc/libbrotlidec when the `system-brotli` feature is on.
//!
//! Resolution order:
//!  1. `WOFF2_BROTLI_LIB_DIR` (+ optional `WOFF2_BROTLI_STATIC=1`), used by
//!     the Makefile build to link the brotli submodule statically;
//!  2. `pkg-config` (libbrotlienc, libbrotlidec);
//!  3. plain `-lbrotlienc -lbrotlidec -lbrotlicommon`.
//!
//! With `system-brotli` disabled (as used for the C++ ABI build), the crate
//! has no link-time dependency on brotli at all: the brotli entry points are
//! passed in as function pointers by libwoff2dec/libwoff2enc.

use std::env;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=WOFF2_BROTLI_LIB_DIR");
    println!("cargo:rerun-if-env-changed=WOFF2_BROTLI_STATIC");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");

    if env::var_os("CARGO_FEATURE_SYSTEM_BROTLI").is_none() {
        return;
    }

    if let Ok(dir) = env::var("WOFF2_BROTLI_LIB_DIR") {
        let kind = if env::var("WOFF2_BROTLI_STATIC").as_deref() == Ok("1") {
            "static"
        } else {
            "dylib"
        };
        println!("cargo:rustc-link-search=native={dir}");
        for lib in ["brotlienc", "brotlidec", "brotlicommon"] {
            println!("cargo:rustc-link-lib={kind}={lib}");
        }
        return;
    }

    let pkg_config = env::var("PKG_CONFIG").unwrap_or_else(|_| "pkg-config".to_string());
    if let Ok(output) = Command::new(pkg_config)
        .args(["--libs", "libbrotlienc", "libbrotlidec"])
        .output()
    {
        if output.status.success() {
            let flags = String::from_utf8_lossy(&output.stdout);
            for flag in flags.split_whitespace() {
                if let Some(dir) = flag.strip_prefix("-L") {
                    println!("cargo:rustc-link-search=native={dir}");
                } else if let Some(lib) = flag.strip_prefix("-l") {
                    println!("cargo:rustc-link-lib={lib}");
                }
            }
            return;
        }
    }

    for lib in ["brotlienc", "brotlidec", "brotlicommon"] {
        println!("cargo:rustc-link-lib={lib}");
    }
}
