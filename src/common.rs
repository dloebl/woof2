//! Definitions shared by the WOFF2 encoder and decoder (`woff2_common.h`).

pub const WOFF2_SIGNATURE: u32 = 0x774f_4632; // "wOF2"

/// Leave the first byte open to store flag_byte.
pub const WOFF2_FLAGS_TRANSFORM: u32 = 1 << 8;

/// TrueType Collection ID string: 'ttcf'.
pub const TTC_FONT_FLAVOR: u32 = 0x7474_6366;

pub const SFNT_HEADER_SIZE: usize = 12;
pub const SFNT_ENTRY_SIZE: usize = 16;

/// Computes the checksum over `buf`, treating a trailing partial word as if
/// it were zero-padded to four bytes.
pub fn compute_ulong_sum(buf: &[u8]) -> u32 {
    let mut checksum: u32 = 0;
    let mut chunks = buf.chunks_exact(4);
    for c in &mut chunks {
        checksum = checksum.wrapping_add(u32::from_be_bytes([c[0], c[1], c[2], c[3]]));
    }
    let rem = chunks.remainder();
    if !rem.is_empty() {
        let mut v: u32 = 0;
        for (i, &b) in rem.iter().enumerate() {
            v |= u32::from(b) << (24 - 8 * i);
        }
        checksum = checksum.wrapping_add(v);
    }
    checksum
}

/// Size of the collection header. 0 if `header_version` indicates this isn't
/// a collection.
pub fn collection_header_size(header_version: u32, num_fonts: u32) -> usize {
    let mut size: usize = 0;
    if header_version == 0x0002_0000 {
        size += 12; // ulDsig{Tag,Length,Offset}
    }
    if header_version == 0x0001_0000 || header_version == 0x0002_0000 {
        // TTCTag, Version, numFonts + OffsetTable[numFonts]. Note that, as in
        // C++, this is computed in 32-bit unsigned arithmetic.
        size += 12u32.wrapping_add(4u32.wrapping_mul(num_fonts)) as usize;
    }
    size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum() {
        assert_eq!(compute_ulong_sum(&[]), 0);
        assert_eq!(compute_ulong_sum(&[1, 2, 3, 4]), 0x01020304);
        assert_eq!(compute_ulong_sum(&[1, 2, 3, 4, 5]), 0x06020304);
        assert_eq!(compute_ulong_sum(&[0xff; 8]), 0xffff_fffe);
    }

    #[test]
    fn collection_header() {
        assert_eq!(collection_header_size(0, 3), 0);
        assert_eq!(collection_header_size(0x0001_0000, 3), 24);
        assert_eq!(collection_header_size(0x0002_0000, 3), 36);
    }
}
