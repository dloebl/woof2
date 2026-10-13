//! Optional diagnostics mirroring the C++ `FONT_COMPRESSION_BIN` messages.

/// Prints a diagnostic to stderr when the `noisy-logging` feature is enabled.
/// The messages are identical to the ones printed by the C++ library when it
/// was compiled with `-DFONT_COMPRESSION_BIN` (CMake `NOISY_LOGGING=ON`).
macro_rules! noisy {
    ($($arg:tt)*) => {{
        #[cfg(feature = "noisy-logging")]
        {
            use std::io::Write as _;
            let mut stderr = std::io::stderr().lock();
            let _ = write!(stderr, $($arg)*);
            let _ = stderr.flush();
        }
        // Type-check (and "use") the arguments either way.
        #[cfg(not(feature = "noisy-logging"))]
        if false {
            let _ = format_args!($($arg)*);
        }
    }};
}
