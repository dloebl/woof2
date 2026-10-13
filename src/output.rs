//! Output interface for WOFF2 decoding (`woff2/output.h`).

/// Suggested max size for output.
pub const DEFAULT_MAX_SIZE: usize = 128 * 1024 * 1024;

/// Output interface for the woff2 decoding, equivalent to `woff2::WOFF2Out`.
///
/// Writes to arbitrary offsets are supported to facilitate updating offset
/// table and checksums after tables are ready. Reading the current size is
/// supported so a 'loca' table can be built up while writing glyphs.
pub trait Woff2Out {
    /// Appends `buf`. Returns true if all was written.
    fn write(&mut self, buf: &[u8]) -> bool;

    /// Writes `buf` at `offset`. Returns true if all was written.
    fn write_at(&mut self, buf: &[u8], offset: usize) -> bool;

    /// The current size of the output.
    fn size(&mut self) -> usize;
}

/// Expanding memory block, equivalent to `woff2::WOFF2StringOut`.
#[derive(Debug)]
pub struct StringOut<'a> {
    buf: &'a mut Vec<u8>,
    max_size: usize,
    offset: usize,
}

impl<'a> StringOut<'a> {
    /// Creates a writer that writes its data to `buf`, which will grow to at
    /// most `max_size` bytes. `buf` may be pre-sized or empty.
    pub fn new(buf: &'a mut Vec<u8>) -> Self {
        StringOut {
            buf,
            max_size: DEFAULT_MAX_SIZE,
            offset: 0,
        }
    }

    pub fn max_size(&self) -> usize {
        self.max_size
    }

    pub fn set_max_size(&mut self, max_size: usize) {
        self.max_size = max_size;
        if self.offset > self.max_size {
            self.offset = self.max_size;
        }
    }
}

impl Woff2Out for StringOut<'_> {
    fn write(&mut self, buf: &[u8]) -> bool {
        self.write_at(buf, self.offset)
    }

    fn write_at(&mut self, buf: &[u8], offset: usize) -> bool {
        let n = buf.len();
        if offset > self.max_size || n > self.max_size - offset {
            return false;
        }
        if offset + n > self.buf.len() {
            self.buf.resize(offset + n, 0);
        }
        self.buf[offset..offset + n].copy_from_slice(buf);
        self.offset = self.offset.max(offset + n);
        true
    }

    fn size(&mut self) -> usize {
        self.offset
    }
}

/// Fixed memory block, equivalent to `woff2::WOFF2MemoryOut`.
#[derive(Debug)]
pub struct MemoryOut<'a> {
    buf: &'a mut [u8],
    offset: usize,
}

impl<'a> MemoryOut<'a> {
    pub fn new(buf: &'a mut [u8]) -> Self {
        MemoryOut { buf, offset: 0 }
    }
}

impl Woff2Out for MemoryOut<'_> {
    fn write(&mut self, buf: &[u8]) -> bool {
        self.write_at(buf, self.offset)
    }

    fn write_at(&mut self, buf: &[u8], offset: usize) -> bool {
        let n = buf.len();
        if offset > self.buf.len() || n > self.buf.len() - offset {
            return false;
        }
        self.buf[offset..offset + n].copy_from_slice(buf);
        self.offset = self.offset.max(offset + n);
        true
    }

    fn size(&mut self) -> usize {
        self.offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn string_out() {
        let mut v = Vec::new();
        let mut out = StringOut::new(&mut v);
        assert!(out.write(b"abc"));
        assert!(out.write_at(b"Z", 5));
        assert_eq!(out.size(), 6);
        out.set_max_size(7);
        assert!(!out.write(b"xy"));
        assert!(out.write(b"x"));
        assert_eq!(v, b"abc\0\0Zx");
    }

    #[test]
    fn memory_out() {
        let mut v = [0u8; 4];
        let mut out = MemoryOut::new(&mut v);
        assert!(out.write(b"ab"));
        assert!(!out.write(b"abc"));
        assert!(out.write_at(b"cd", 2));
        assert_eq!(out.size(), 4);
        assert_eq!(&v, b"abcd");
    }
}
