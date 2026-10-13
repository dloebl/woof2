//! File helpers shared by the command line tools (`file.h`).
#![allow(dead_code)]

use std::ffi::OsString;
use std::path::PathBuf;

/// The command line arguments as raw bytes.
pub fn args() -> Vec<Vec<u8>> {
    std::env::args_os().map(os_to_bytes).collect()
}

#[cfg(unix)]
fn os_to_bytes(s: OsString) -> Vec<u8> {
    use std::os::unix::ffi::OsStringExt;
    s.into_vec()
}

#[cfg(not(unix))]
fn os_to_bytes(s: OsString) -> Vec<u8> {
    s.to_string_lossy().into_owned().into_bytes()
}

#[cfg(unix)]
fn bytes_to_path(b: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(b))
}

#[cfg(not(unix))]
fn bytes_to_path(b: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(b).into_owned())
}

/// `filename.substr(0, filename.find_last_of(".")) + ext`
pub fn replace_extension(filename: &[u8], ext: &[u8]) -> Vec<u8> {
    let stem = match filename.iter().rposition(|&c| c == b'.') {
        Some(pos) => &filename[..pos],
        None => filename,
    };
    let mut out = stem.to_vec();
    out.extend_from_slice(ext);
    out
}

/// Reads the whole file; an unreadable file yields empty contents.
pub fn get_file_content(filename: &[u8]) -> Vec<u8> {
    std::fs::read(bytes_to_path(filename)).unwrap_or_default()
}

/// Writes the whole file; errors are ignored.
pub fn set_file_contents(filename: &[u8], data: &[u8]) {
    let _ = std::fs::write(bytes_to_path(filename), data);
}

/// Mimics C stdio buffering of `stdout`, which matters for the relative order
/// of stdout and stderr output when both are redirected to the same file:
/// line-buffered on a terminal, fully buffered (written at exit) otherwise.
pub struct Stdout {
    buf: Vec<u8>,
    tty: bool,
}

impl Stdout {
    pub fn new() -> Self {
        Stdout {
            buf: Vec::new(),
            tty: stdout_is_terminal(),
        }
    }

    pub fn print(&mut self, data: &[u8]) {
        if self.tty {
            use std::io::Write;
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(data);
            let _ = out.flush();
        } else {
            self.buf.extend_from_slice(data);
        }
    }
}

impl Drop for Stdout {
    fn drop(&mut self) {
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let _ = out.write_all(&self.buf);
        let _ = out.flush();
    }
}

#[cfg(unix)]
fn stdout_is_terminal() -> bool {
    extern "C" {
        fn isatty(fd: std::os::raw::c_int) -> std::os::raw::c_int;
    }
    // SAFETY: isatty has no preconditions.
    unsafe { isatty(1) == 1 }
}

#[cfg(not(unix))]
fn stdout_is_terminal() -> bool {
    false
}
