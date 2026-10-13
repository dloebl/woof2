//! WOFF2 variable length integer types: 255UInt16 and UIntBase128.

use crate::buffer::Buffer;

/// Number of bytes needed to encode `value` as 255UInt16.
pub fn size_255_ushort(value: u16) -> usize {
    if value < 253 {
        1
    } else if value < 762 {
        2
    } else {
        3
    }
}

/// Encodes `value` as 255UInt16 into `out`, returning the number of bytes
/// used. Mirrors the C++ `Write255UShort(std::vector<uint8_t>*, int)`,
/// including its truncation behavior for out-of-range values.
pub fn encode_255_ushort(value: i32, out: &mut [u8; 3]) -> usize {
    if value < 253 {
        out[0] = value as u8;
        1
    } else if value < 506 {
        out[0] = 255;
        out[1] = (value - 253) as u8;
        2
    } else if value < 762 {
        out[0] = 254;
        out[1] = (value - 506) as u8;
        2
    } else {
        out[0] = 253;
        out[1] = (value >> 8) as u8;
        out[2] = (value & 0xff) as u8;
        3
    }
}

/// Appends `value` encoded as 255UInt16.
pub fn write_255_ushort(out: &mut Vec<u8>, value: i32) {
    let mut tmp = [0u8; 3];
    let n = encode_255_ushort(value, &mut tmp);
    out.extend_from_slice(&tmp[..n]);
}

/// Reads a 255UInt16 (section 6.1.1 of the MicroType Express draft spec).
///
/// Like the C++ version, a failure after the first byte has been consumed
/// leaves the buffer advanced past that byte.
pub fn read_255_ushort(buf: &mut Buffer<'_>) -> Option<u32> {
    const WORD_CODE: u8 = 253;
    const ONE_MORE_BYTE_CODE2: u8 = 254;
    const ONE_MORE_BYTE_CODE1: u8 = 255;
    const LOWEST_U_CODE: u32 = 253;
    let code = buf.read_u8()?;
    match code {
        WORD_CODE => buf.read_u16().map(u32::from),
        ONE_MORE_BYTE_CODE1 => buf.read_u8().map(|r| u32::from(r) + LOWEST_U_CODE),
        ONE_MORE_BYTE_CODE2 => buf.read_u8().map(|r| u32::from(r) + LOWEST_U_CODE * 2),
        _ => Some(u32::from(code)),
    }
}

/// Reads a UIntBase128.
///
/// Like the C++ version, a failure leaves the buffer advanced past the bytes
/// that were consumed.
pub fn read_base128(buf: &mut Buffer<'_>) -> Option<u32> {
    let mut result: u32 = 0;
    for i in 0..5 {
        let code = buf.read_u8()?;
        // Leading zeros are invalid.
        if i == 0 && code == 0x80 {
            return None;
        }
        // If any of the top seven bits are set then we're about to overflow.
        if result & 0xfe00_0000 != 0 {
            return None;
        }
        result = (result << 7) | u32::from(code & 0x7f);
        if code & 0x80 == 0 {
            return Some(result);
        }
    }
    // Make sure not to exceed the size bound
    None
}

/// Number of bytes needed to encode `n` as UIntBase128.
pub fn base128_size(mut n: usize) -> usize {
    let mut size = 1;
    while n >= 128 {
        size += 1;
        n >>= 7;
    }
    size
}

/// Encodes `len` as UIntBase128 into `out`, returning the number of bytes.
pub fn encode_base128(len: usize, out: &mut [u8; 10]) -> usize {
    let size = base128_size(len);
    for (i, slot) in out.iter_mut().enumerate().take(size) {
        let mut b = ((len >> (7 * (size - i - 1))) & 0x7f) as u8;
        if i < size - 1 {
            b |= 0x80;
        }
        *slot = b;
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ushort_255_round_trip() {
        for v in 0..=65535u32 {
            let mut tmp = [0u8; 3];
            let n = encode_255_ushort(v as i32, &mut tmp);
            assert_eq!(n, size_255_ushort(v as u16));
            let mut b = Buffer::new(&tmp[..n]);
            assert_eq!(read_255_ushort(&mut b), Some(v), "value {v}");
        }
    }

    #[test]
    fn base128_round_trip() {
        for &v in &[0usize, 1, 127, 128, 16383, 16384, 0xffff_ffff] {
            let mut tmp = [0u8; 10];
            let n = encode_base128(v, &mut tmp);
            assert_eq!(n, base128_size(v));
            let mut b = Buffer::new(&tmp[..n]);
            assert_eq!(read_base128(&mut b), Some(v as u32));
        }
        // Leading zero.
        assert_eq!(read_base128(&mut Buffer::new(&[0x80, 0x01])), None);
        // Overflow.
        assert_eq!(
            read_base128(&mut Buffer::new(&[0x90, 0x80, 0x80, 0x80, 0x00])),
            None
        );
    }
}
