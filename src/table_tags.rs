//! Font table tags.

const fn tag(t: &[u8; 4]) -> u32 {
    (t[0] as u32) << 24 | (t[1] as u32) << 16 | (t[2] as u32) << 8 | t[3] as u32
}

pub const GLYF_TABLE_TAG: u32 = 0x676c_7966;
pub const HEAD_TABLE_TAG: u32 = 0x6865_6164;
pub const LOCA_TABLE_TAG: u32 = 0x6c6f_6361;
pub const DSIG_TABLE_TAG: u32 = 0x4453_4947;
pub const CFF_TABLE_TAG: u32 = 0x4346_4620;
pub const HMTX_TABLE_TAG: u32 = 0x686d_7478;
pub const HHEA_TABLE_TAG: u32 = 0x6868_6561;
pub const MAXP_TABLE_TAG: u32 = 0x6d61_7870;

/// The 63 "known" table tags that can be encoded with a 6-bit index in the
/// WOFF2 table directory. Note that the byte order is big-endian.
pub const KNOWN_TAGS: [u32; 63] = [
    tag(b"cmap"), // 0
    tag(b"head"), // 1
    tag(b"hhea"), // 2
    tag(b"hmtx"), // 3
    tag(b"maxp"), // 4
    tag(b"name"), // 5
    tag(b"OS/2"), // 6
    tag(b"post"), // 7
    tag(b"cvt "), // 8
    tag(b"fpgm"), // 9
    tag(b"glyf"), // 10
    tag(b"loca"), // 11
    tag(b"prep"), // 12
    tag(b"CFF "), // 13
    tag(b"VORG"), // 14
    tag(b"EBDT"), // 15
    tag(b"EBLC"), // 16
    tag(b"gasp"), // 17
    tag(b"hdmx"), // 18
    tag(b"kern"), // 19
    tag(b"LTSH"), // 20
    tag(b"PCLT"), // 21
    tag(b"VDMX"), // 22
    tag(b"vhea"), // 23
    tag(b"vmtx"), // 24
    tag(b"BASE"), // 25
    tag(b"GDEF"), // 26
    tag(b"GPOS"), // 27
    tag(b"GSUB"), // 28
    tag(b"EBSC"), // 29
    tag(b"JSTF"), // 30
    tag(b"MATH"), // 31
    tag(b"CBDT"), // 32
    tag(b"CBLC"), // 33
    tag(b"COLR"), // 34
    tag(b"CPAL"), // 35
    tag(b"SVG "), // 36
    tag(b"sbix"), // 37
    tag(b"acnt"), // 38
    tag(b"avar"), // 39
    tag(b"bdat"), // 40
    tag(b"bloc"), // 41
    tag(b"bsln"), // 42
    tag(b"cvar"), // 43
    tag(b"fdsc"), // 44
    tag(b"feat"), // 45
    tag(b"fmtx"), // 46
    tag(b"fvar"), // 47
    tag(b"gvar"), // 48
    tag(b"hsty"), // 49
    tag(b"just"), // 50
    tag(b"lcar"), // 51
    tag(b"mort"), // 52
    tag(b"morx"), // 53
    tag(b"opbd"), // 54
    tag(b"prop"), // 55
    tag(b"trak"), // 56
    tag(b"Zapf"), // 57
    tag(b"Silf"), // 58
    tag(b"Glat"), // 59
    tag(b"Gloc"), // 60
    tag(b"Feat"), // 61
    tag(b"Sill"), // 62
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_match_known_tags() {
        assert_eq!(KNOWN_TAGS[10], GLYF_TABLE_TAG);
        assert_eq!(KNOWN_TAGS[1], HEAD_TABLE_TAG);
        assert_eq!(KNOWN_TAGS[11], LOCA_TABLE_TAG);
        assert_eq!(KNOWN_TAGS[13], CFF_TABLE_TAG);
        assert_eq!(KNOWN_TAGS[3], HMTX_TABLE_TAG);
        assert_eq!(KNOWN_TAGS[2], HHEA_TABLE_TAG);
        assert_eq!(KNOWN_TAGS[4], MAXP_TABLE_TAG);
        assert_eq!(DSIG_TABLE_TAG, tag(b"DSIG"));
    }
}
