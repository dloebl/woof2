//! Font normalization (`normalize.cc`).
//!
//! Since the WOFF 2.0 decoder creates font files in normalized form, the
//! WOFF 2.0 conversion is guaranteed to be lossless (in a bitwise sense) only
//! for normalized font files.

use crate::common::{collection_header_size, compute_ulong_sum, SFNT_ENTRY_SIZE, SFNT_HEADER_SIZE};
use crate::store::{log2_floor, round4_i32, round4_u32, round4_usize};
use crate::table_tags::{GLYF_TABLE_TAG, HEAD_TABLE_TAG, LOCA_TABLE_TAG};

use super::font::{
    get_glyph_data, num_glyphs, remove_digital_signature, resolve, DataRef, Font, FontCollection,
};
use super::glyph::{read_glyph, store_glyph, Glyph};

fn store_loca(index_fmt: i32, value: u32, offset: &mut usize, dst: &mut [u8]) {
    if index_fmt == 0 {
        let v = (value >> 1) as i32;
        dst[*offset] = (v >> 8) as u8;
        dst[*offset + 1] = v as u8;
        *offset += 2;
    } else {
        dst[*offset..*offset + 4].copy_from_slice(&value.to_be_bytes());
        *offset += 4;
    }
}

fn write_normalized_loca(index_fmt: i32, num_glyphs: i32, font: &mut Font, input: &[u8]) -> bool {
    let glyph_sz: i32 = if index_fmt == 0 { 2 } else { 4 };
    {
        let loca = font.find_table_mut(LOCA_TABLE_TAG).expect("loca present");
        let loca_buf_size = round4_i32(num_glyphs.wrapping_add(1)).wrapping_mul(glyph_sz);
        loca.buffer.resize(loca_buf_size as usize, 0);
        loca.length = (num_glyphs.wrapping_add(1)).wrapping_mul(glyph_sz) as u32;
    }

    let mut glyf_offset: u32 = 0;
    let mut loca_offset: usize = 0;
    // Glyph data is copied out first, since in general it may live in the
    // very buffer that is being rewritten.
    let mut scratch: Vec<u8> = Vec::new();
    // The loca entries are collected separately and copied into the loca
    // table's buffer at the end (the loca buffer is only read through `data`,
    // which still points at the original loca at this point).
    let mut loca_buf = std::mem::take(&mut font.find_table_mut(LOCA_TABLE_TAG).unwrap().buffer);
    let mut glyf_buf = std::mem::take(&mut font.find_table_mut(GLYF_TABLE_TAG).unwrap().buffer);

    let ok = (|| {
        for i in 0..num_glyphs {
            store_loca(index_fmt, glyf_offset, &mut loca_offset, &mut loca_buf);
            let glyph_data = match get_glyph_data(font, i, input) {
                Some(d) => d,
                None => return false,
            };
            scratch.clear();
            scratch.extend_from_slice(glyph_data);
            let mut glyph = Glyph::default();
            if !scratch.is_empty() && !read_glyph(&scratch, &mut glyph) {
                return false;
            }
            // NOTE: the C++ code computes `buffer.size() - glyf_offset`, which
            // wraps around if rounding up the previous glyph went past the end
            // of the buffer, and then writes out of bounds. Saturating gives
            // the same result wherever the C++ behavior is defined.
            let start = (glyf_offset as usize).min(glyf_buf.len());
            let mut glyf_dst_size = match store_glyph(&glyph, &mut glyf_buf[start..]) {
                Some(n) => n,
                None => return false,
            };
            glyf_dst_size = round4_usize(glyf_dst_size);
            if glyf_dst_size > u32::MAX as usize
                || glyf_offset.wrapping_add(glyf_dst_size as u32) < glyf_offset
                || (index_fmt == 0 && glyf_offset as usize + glyf_dst_size >= (1usize << 17))
            {
                return false;
            }
            glyf_offset = glyf_offset.wrapping_add(glyf_dst_size as u32);
        }
        true
    })();

    if !ok {
        font.find_table_mut(LOCA_TABLE_TAG).unwrap().buffer = loca_buf;
        font.find_table_mut(GLYF_TABLE_TAG).unwrap().buffer = glyf_buf;
        return false;
    }

    store_loca(index_fmt, glyf_offset, &mut loca_offset, &mut loca_buf);

    let glyf = font.find_table_mut(GLYF_TABLE_TAG).unwrap();
    glyf_buf.resize(glyf_offset as usize, 0);
    glyf.buffer = glyf_buf;
    glyf.data = if glyf_offset != 0 {
        DataRef::Buffer
    } else {
        DataRef::Null
    };
    glyf.length = glyf_offset;
    let loca = font.find_table_mut(LOCA_TABLE_TAG).unwrap();
    loca.buffer = loca_buf;
    loca.data = if loca_offset != 0 {
        DataRef::Buffer
    } else {
        DataRef::Null
    };

    true
}

fn make_editable_buffer(font: &mut Font, table_tag: u32, input: &[u8]) -> bool {
    let table = match font.find_table_mut(table_tag) {
        Some(t) => t,
        None => return false,
    };
    if table.is_reused() {
        return true;
    }
    let sz = round4_u32(table.length) as usize;
    let copy: Vec<u8> = table.data(input).to_vec();
    table.buffer.resize(sz, 0);
    let n = copy.len().min(table.length as usize);
    table.buffer[..n].copy_from_slice(&copy[..n]);
    for b in &mut table.buffer[n.min(sz)..] {
        *b = 0;
    }
    table.data = DataRef::Buffer;
    true
}

/// Parses each of the glyphs in the font and writes them again to the glyf
/// table in normalized form. Changes the loca table accordingly.
pub fn normalize_glyphs(font: &mut Font, input: &[u8]) -> bool {
    let (head, glyf, loca) = (
        font.find_table(HEAD_TABLE_TAG),
        font.find_table(GLYF_TABLE_TAG),
        font.find_table(LOCA_TABLE_TAG),
    );
    let head = match head {
        Some(h) => h,
        None => return false,
    };
    // If you don't have glyf/loca this transform isn't very interesting
    if loca.is_none() && glyf.is_none() {
        return true;
    }
    // It would be best if you didn't have just one of glyf/loca
    let (glyf, loca) = match (glyf, loca) {
        (Some(g), Some(l)) => (g, l),
        _ => return false,
    };
    // Must share neither or both loca & glyf
    if loca.is_reused() != glyf.is_reused() {
        return false;
    }
    if loca.is_reused() {
        return true;
    }

    let mut index_fmt = i32::from(head.data(input).get(51).copied().unwrap_or(0));
    let num_glyphs = num_glyphs(font, input);

    // We need to allocate a bit more than its original length for the
    // normalized glyf table, since it can happen that the glyphs in the
    // original table are 2-byte aligned, while in the normalized table they
    // are 4-byte aligned. That gives a maximum of 2 bytes increase per glyph.
    // However, there is no theoretical guarantee that the total size of the
    // flags plus the coordinates is the smallest possible in the normalized
    // version, so we have to allow some general overhead.
    let glyf_length = font.find_table(GLYF_TABLE_TAG).unwrap().length;
    let max_normalized_glyf_size =
        (1.1 * f64::from(glyf_length) + f64::from(num_glyphs.wrapping_mul(2))) as usize;

    {
        let glyf = font.find_table_mut(GLYF_TABLE_TAG).unwrap();
        if glyf.buffer.try_reserve(max_normalized_glyf_size).is_err() {
            return false;
        }
        glyf.buffer.resize(max_normalized_glyf_size, 0);
    }

    // if we can't write a loca using short's (index_fmt 0)
    // try again using longs (index_fmt 1)
    if !write_normalized_loca(index_fmt, num_glyphs, font, input) {
        if index_fmt != 0 {
            return false;
        }

        // Rewrite loca with 4-byte entries & update head to match
        index_fmt = 1;
        if !write_normalized_loca(index_fmt, num_glyphs, font, input) {
            return false;
        }
        let head = font.find_table_mut(HEAD_TABLE_TAG).unwrap();
        // NOTE: for a 'head' reused from another font in a collection, the C++
        // code writes to an empty std::vector here (undefined behavior).
        if head.buffer.len() <= 51 {
            return false;
        }
        head.buffer[51] = 1;
    }

    true
}

/// Changes the offset fields of the table headers so that the data for the
/// tables will be written in order of increasing tag values, without any
/// gaps other than the 4-byte padding.
pub fn normalize_offsets(font: &mut Font) -> bool {
    let mut offset: u32 = 12u32.wrapping_add(16u32.wrapping_mul(u32::from(font.num_tables)));
    for tag in font.output_ordered_tags() {
        let table = font.tables.entry(tag).or_default();
        table.offset = offset;
        offset = offset.wrapping_add(round4_u32(table.length));
    }
    true
}

fn compute_header_checksum(fonts: &[Font], font_index: usize) -> u32 {
    let font = &fonts[font_index];
    let num_tables = u32::from(font.num_tables);
    let mut checksum: u32 = font.flavor;
    let max_pow2: u16 = if num_tables != 0 {
        log2_floor(num_tables) as u16
    } else {
        0
    };
    let search_range: u16 = if max_pow2 != 0 {
        (1u32 << (u32::from(max_pow2) + 4)) as u16
    } else {
        0
    };
    let range_shift: u16 = ((num_tables << 4) as u16).wrapping_sub(search_range);
    checksum = checksum.wrapping_add((num_tables << 16) | u32::from(search_range));
    checksum = checksum.wrapping_add((u32::from(max_pow2) << 16) | u32::from(range_shift));
    for &tag in font.tables.keys() {
        let orig = resolve(fonts, font_index, tag);
        let table = match fonts[orig].tables.get(&tag) {
            Some(t) => t,
            None => continue,
        };
        checksum = checksum.wrapping_add(table.tag);
        checksum = checksum.wrapping_add(table.checksum);
        checksum = checksum.wrapping_add(table.offset);
        checksum = checksum.wrapping_add(table.length);
    }
    checksum
}

/// Changes the checksum fields of the table headers and the checksum field
/// of the head table so that it matches the current data.
pub fn fix_checksums(fonts: &mut [Font], font_index: usize, input: &[u8]) -> bool {
    if fonts[font_index].find_table(HEAD_TABLE_TAG).is_none() {
        return false;
    }
    let head_font = resolve(fonts, font_index, HEAD_TABLE_TAG);
    {
        let head = match fonts[head_font].find_table_mut(HEAD_TABLE_TAG) {
            Some(h) => h,
            None => return false,
        };
        if head.length < 12 || head.buffer.len() < 12 {
            return false;
        }
        head.buffer[8..12].copy_from_slice(&0u32.to_be_bytes());
    }

    let mut file_checksum: u32 = 0;
    let tags: Vec<u32> = fonts[font_index].tables.keys().copied().collect();
    for tag in tags {
        let orig = resolve(fonts, font_index, tag);
        let table = match fonts[orig].tables.get_mut(&tag) {
            Some(t) => t,
            None => return false,
        };
        table.checksum = compute_ulong_sum(table.data(input));
        file_checksum = file_checksum.wrapping_add(table.checksum);
    }

    file_checksum = file_checksum.wrapping_add(compute_header_checksum(fonts, font_index));
    let head = fonts[head_font].find_table_mut(HEAD_TABLE_TAG).unwrap();
    head.buffer[8..12].copy_from_slice(&0xb1b0_afbau32.wrapping_sub(file_checksum).to_be_bytes());

    true
}

fn mark_transformed(fonts: &mut [Font], font_index: usize, input: &[u8]) -> bool {
    if fonts[font_index].find_table(HEAD_TABLE_TAG).is_none() {
        return false;
    }
    let head_font = resolve(fonts, font_index, HEAD_TABLE_TAG);
    let head = match fonts[head_font].find_table_mut(HEAD_TABLE_TAG) {
        Some(h) => h,
        None => return false,
    };
    if head.length < 17 {
        return false;
    }
    // set bit 11 of head table 'flags' to indicate that font has undergone
    // lossless modifying transform
    let head_flags = head.data(input).get(16).copied().unwrap_or(0);
    if head.buffer.len() <= 16 {
        return false;
    }
    head.buffer[16] = head_flags | 0x08;
    true
}

fn normalize_without_fixing_checksums(fonts: &mut [Font], font_index: usize, input: &[u8]) -> bool {
    make_editable_buffer(&mut fonts[font_index], HEAD_TABLE_TAG, input)
        && remove_digital_signature(&mut fonts[font_index])
        && mark_transformed(fonts, font_index, input)
        && normalize_glyphs(&mut fonts[font_index], input)
        && normalize_offsets(&mut fonts[font_index])
}

/// Performs all of the normalization steps on a single font.
pub fn normalize_font(fonts: &mut [Font], font_index: usize, input: &[u8]) -> bool {
    normalize_without_fixing_checksums(fonts, font_index, input)
        && fix_checksums(fonts, font_index, input)
}

/// Normalizes a font or a font collection.
pub fn normalize_font_collection(font_collection: &mut FontCollection, input: &[u8]) -> bool {
    if font_collection.fonts.len() == 1 {
        return normalize_font(&mut font_collection.fonts, 0, input);
    }

    let mut offset: u32 = collection_header_size(
        font_collection.header_version,
        font_collection.fonts.len() as u32,
    ) as u32;
    for i in 0..font_collection.fonts.len() {
        if !normalize_without_fixing_checksums(&mut font_collection.fonts, i, input) {
            noisy!("Font normalization failed.\n");
            return false;
        }
        let num_tables = usize::from(font_collection.fonts[i].num_tables);
        offset = offset.wrapping_add((SFNT_HEADER_SIZE + SFNT_ENTRY_SIZE * num_tables) as u32);
    }

    // Start table offsets after TTC Header and Sfnt Headers
    for i in 0..font_collection.fonts.len() {
        for tag in font_collection.fonts[i].output_ordered_tags() {
            let reuse_of = font_collection.fonts[i]
                .tables
                .entry(tag)
                .or_default()
                .reuse_of;
            match reuse_of {
                Some(orig) => {
                    let orig_offset = font_collection.fonts[orig]
                        .tables
                        .get(&tag)
                        .map(|t| t.offset)
                        .unwrap_or(0);
                    font_collection.fonts[i]
                        .tables
                        .get_mut(&tag)
                        .unwrap()
                        .offset = orig_offset;
                }
                None => {
                    let table = font_collection.fonts[i].tables.get_mut(&tag).unwrap();
                    table.offset = offset;
                    offset = offset.wrapping_add(round4_u32(table.length));
                }
            }
        }
    }

    // Now we can fix the checksums
    for i in 0..font_collection.fonts.len() {
        if !fix_checksums(&mut font_collection.fonts, i, input) {
            noisy!("Failed to fix checksums\n");
            return false;
        }
    }

    true
}
