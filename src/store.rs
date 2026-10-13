//! Helpers for storing big-endian integers and for 4-byte rounding
//! (`store_bytes.h` / `round.h`).

/// Stores `x` big-endian at `dst[offset..offset + 4]`, returns the new offset.
#[inline]
pub fn store_u32(dst: &mut [u8], offset: usize, x: u32) -> usize {
    dst[offset..offset + 4].copy_from_slice(&x.to_be_bytes());
    offset + 4
}

/// Stores the low 16 bits of `x` big-endian, returns the new offset.
#[inline]
pub fn store16(dst: &mut [u8], offset: usize, x: i32) -> usize {
    dst[offset] = (x >> 8) as u8;
    dst[offset + 1] = x as u8;
    offset + 2
}

/// Rounds up to a multiple of 4, unless that would overflow (C++ `Round4`).
#[inline]
pub fn round4_u32(value: u32) -> u32 {
    if u32::MAX - value < 3 {
        value
    } else {
        (value + 3) & !3
    }
}

#[inline]
pub fn round4_u64(value: u64) -> u64 {
    if u64::MAX - value < 3 {
        value
    } else {
        (value + 3) & !3
    }
}

#[inline]
pub fn round4_usize(value: usize) -> usize {
    if usize::MAX - value < 3 {
        value
    } else {
        (value + 3) & !3
    }
}

/// `Round4<int>`; only ever used with non-negative values.
#[inline]
pub fn round4_i32(value: i32) -> i32 {
    if i32::MAX.wrapping_sub(value) < 3 {
        value
    } else {
        value.wrapping_add(3) & !3
    }
}

/// `Log2Floor` from `port.h`.
#[inline]
pub fn log2_floor(n: u32) -> i32 {
    if n == 0 {
        -1
    } else {
        31 ^ n.leading_zeros() as i32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounding() {
        assert_eq!(round4_u32(0), 0);
        assert_eq!(round4_u32(1), 4);
        assert_eq!(round4_u32(4), 4);
        assert_eq!(round4_u32(u32::MAX - 2), u32::MAX - 2);
        assert_eq!(round4_u32(u32::MAX - 3), u32::MAX - 3);
        assert_eq!(round4_u32(u32::MAX - 4), u32::MAX - 3);
        assert_eq!(round4_usize(usize::MAX), usize::MAX);
        assert_eq!(log2_floor(1), 0);
        assert_eq!(log2_floor(17), 4);
    }
}
