//! Bounds-checked big-endian reader, equivalent to `woff2::Buffer`.

/// Reads from a byte slice while checking for out-of-bounds accesses.
///
/// Every read either succeeds and advances the offset, or fails (returns
/// `None`/`false`) and leaves the offset untouched, exactly like the C++
/// `woff2::Buffer` class.
#[derive(Clone, Copy, Debug)]
pub struct Buffer<'a> {
    buffer: &'a [u8],
    offset: usize,
}

/// Reads larger than this are always rejected (matches the C++ limit).
const MAX_READ: usize = 1024 * 1024 * 1024;

impl<'a> Buffer<'a> {
    pub fn new(buffer: &'a [u8]) -> Self {
        Buffer { buffer, offset: 0 }
    }

    /// Creates a buffer positioned at `offset` (which must be `<= len`).
    pub fn with_offset(buffer: &'a [u8], offset: usize) -> Self {
        debug_assert!(offset <= buffer.len());
        Buffer { buffer, offset }
    }

    #[inline]
    fn has(&self, n: usize) -> bool {
        let len = self.buffer.len();
        len >= n && self.offset <= len - n
    }

    /// Skips `n` bytes.
    pub fn skip(&mut self, n: usize) -> bool {
        self.read(n).is_some()
    }

    /// Reads `n` bytes, returning a view of them.
    pub fn read(&mut self, n: usize) -> Option<&'a [u8]> {
        if n > MAX_READ {
            return None;
        }
        if !self.has(n) {
            return None;
        }
        let s = &self.buffer[self.offset..self.offset + n];
        self.offset += n;
        Some(s)
    }

    #[inline]
    pub fn read_u8(&mut self) -> Option<u8> {
        if !self.has(1) {
            return None;
        }
        let v = self.buffer[self.offset];
        self.offset += 1;
        Some(v)
    }

    #[inline]
    pub fn read_u16(&mut self) -> Option<u16> {
        if !self.has(2) {
            return None;
        }
        let b = &self.buffer[self.offset..self.offset + 2];
        self.offset += 2;
        Some(u16::from_be_bytes([b[0], b[1]]))
    }

    #[inline]
    pub fn read_s16(&mut self) -> Option<i16> {
        self.read_u16().map(|v| v as i16)
    }

    #[inline]
    pub fn read_u24(&mut self) -> Option<u32> {
        if !self.has(3) {
            return None;
        }
        let b = &self.buffer[self.offset..self.offset + 3];
        self.offset += 3;
        Some(u32::from(b[0]) << 16 | u32::from(b[1]) << 8 | u32::from(b[2]))
    }

    #[inline]
    pub fn read_u32(&mut self) -> Option<u32> {
        if !self.has(4) {
            return None;
        }
        let b = &self.buffer[self.offset..self.offset + 4];
        self.offset += 4;
        Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
    }

    #[inline]
    pub fn read_s32(&mut self) -> Option<i32> {
        self.read_u32().map(|v| v as i32)
    }

    /// The underlying data (from the very beginning, not the current offset).
    pub fn buffer(&self) -> &'a [u8] {
        self.buffer
    }

    /// The unread remainder of the data.
    pub fn remaining(&self) -> &'a [u8] {
        &self.buffer[self.offset..]
    }

    pub fn offset(&self) -> usize {
        self.offset
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    pub fn set_offset(&mut self, new_offset: usize) -> bool {
        if new_offset > self.buffer.len() {
            return false;
        }
        self.offset = new_offset;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_are_bounds_checked() {
        let data = [1u8, 2, 3, 4, 5];
        let mut b = Buffer::new(&data);
        assert_eq!(b.read_u16(), Some(0x0102));
        assert_eq!(b.read_u32(), None);
        assert_eq!(b.offset(), 2);
        assert_eq!(b.read_u24(), Some(0x030405));
        assert_eq!(b.read_u8(), None);
        assert!(b.skip(0));
        assert!(!b.skip(1));
        assert!(b.set_offset(5));
        assert!(!b.set_offset(6));
    }
}
