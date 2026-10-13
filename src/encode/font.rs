//! Data model for a font file in sfnt format (`font.cc`).
//!
//! In C++ every table holds a raw `data` pointer (into the input font or into
//! the table's own mutable `buffer`) plus a `length` that can be changed
//! independently. That is modelled here with [`DataRef`]; views are always
//! clamped to the memory that actually exists, which is equivalent for every
//! access pattern the encoder performs.

use std::collections::BTreeMap;

use crate::buffer::Buffer;
use crate::common::TTC_FONT_FLAVOR;
use crate::table_tags::{DSIG_TABLE_TAG, GLYF_TABLE_TAG, HEAD_TABLE_TAG, LOCA_TABLE_TAG};

/// Where a table's `data` pointer points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DataRef {
    /// `NULL`.
    #[default]
    Null,
    /// Into the input font data, at the given offset.
    Input(usize),
    /// To the start of the table's own `buffer`.
    Buffer,
}

#[derive(Clone, Debug, Default)]
pub struct FontTable {
    pub tag: u32,
    pub checksum: u32,
    pub offset: u32,
    pub length: u32,
    pub data: DataRef,
    /// Buffer used to mutate the data before writing out.
    pub buffer: Vec<u8>,
    /// If we've seen this tag/offset before (in a TTC), the index of the font
    /// holding the first table we saw it in (with the same tag).
    pub reuse_of: Option<usize>,
    pub flag_byte: u8,
}

impl FontTable {
    pub fn is_reused(&self) -> bool {
        self.reuse_of.is_some()
    }

    /// The bytes `data[0..length]`, clamped to the memory that exists.
    pub fn data<'a>(&'a self, input: &'a [u8]) -> &'a [u8] {
        let len = self.length as usize;
        match self.data {
            DataRef::Null => &[],
            DataRef::Input(off) => {
                let avail = &input[off.min(input.len())..];
                &avail[..len.min(avail.len())]
            }
            DataRef::Buffer => &self.buffer[..len.min(self.buffer.len())],
        }
    }
}

/// Represents an sfnt font file. Only the table directory is parsed.
#[derive(Clone, Debug, Default)]
pub struct Font {
    pub flavor: u32,
    pub num_tables: u16,
    pub tables: BTreeMap<u32, FontTable>,
}

impl Font {
    pub fn find_table(&self, tag: u32) -> Option<&FontTable> {
        self.tables.get(&tag)
    }

    pub fn find_table_mut(&mut self, tag: u32) -> Option<&mut FontTable> {
        self.tables.get_mut(&tag)
    }

    /// Tags in the order the tables are written: alphabetically, but with
    /// loca immediately after glyf. Transformed tables are excluded.
    pub fn output_ordered_tags(&self) -> Vec<u32> {
        let mut output_order: Vec<u32> = self
            .tables
            .values()
            // This is a transformed table, we will write it together with the
            // original version.
            .filter(|t| t.tag & 0x8080_8080 == 0)
            .map(|t| t.tag)
            .collect();

        // Alphabetize then put loca immediately after glyf
        let glyf_loc = output_order.iter().position(|&t| t == GLYF_TABLE_TAG);
        let loca_loc = output_order.iter().position(|&t| t == LOCA_TABLE_TAG);
        if let (Some(_), Some(loca_loc)) = (glyf_loc, loca_loc) {
            output_order.remove(loca_loc);
            let glyf_loc = output_order
                .iter()
                .position(|&t| t == GLYF_TABLE_TAG)
                .expect("glyf present");
            output_order.insert(glyf_loc + 1, LOCA_TABLE_TAG);
        }

        output_order
    }
}

/// Accommodates both singular (OTF, TTF) and collection (TTC) fonts.
#[derive(Clone, Debug, Default)]
pub struct FontCollection {
    pub flavor: u32,
    pub header_version: u32,
    pub fonts: Vec<Font>,
}

/// Resolves a (possibly reused) table to the font index holding the original.
pub fn resolve(fonts: &[Font], font_index: usize, tag: u32) -> usize {
    match fonts[font_index].tables.get(&tag).and_then(|t| t.reuse_of) {
        Some(orig) => orig,
        None => font_index,
    }
}

fn read_true_type_font(file: &mut Buffer<'_>, len: usize, font: &mut Font) -> bool {
    // We don't care about the search_range, entry_selector and range_shift
    // fields, they will always be computed upon writing the font.
    match file.read_u16() {
        Some(v) => font.num_tables = v,
        None => return false,
    }
    if !file.skip(6) {
        return false;
    }

    let mut intervals: BTreeMap<u32, u32> = BTreeMap::new();
    for _ in 0..font.num_tables {
        let mut table = FontTable::default();
        match (
            file.read_u32(),
            file.read_u32(),
            file.read_u32(),
            file.read_u32(),
        ) {
            (Some(tag), Some(checksum), Some(offset), Some(length)) => {
                table.tag = tag;
                table.checksum = checksum;
                table.offset = offset;
                table.length = length;
            }
            _ => return false,
        }
        if (table.offset & 3) != 0
            || table.length as usize > len
            || len - (table.length as usize) < table.offset as usize
        {
            return false;
        }
        intervals.insert(table.offset, table.length);
        table.data = DataRef::Input(table.offset as usize);
        if font.tables.contains_key(&table.tag) {
            return false;
        }
        font.tables.insert(table.tag, table);
    }

    // Check that tables are non-overlapping.
    let mut last_offset: u32 = (12u64 + 16u64 * u64::from(font.num_tables)) as u32;
    for (&off, &length) in &intervals {
        if off < last_offset || off.wrapping_add(length) < off {
            return false;
        }
        last_offset = off.wrapping_add(length);
    }

    // Sanity check key tables
    if let Some(head) = font.find_table(HEAD_TABLE_TAG) {
        if head.length < 52 {
            return false;
        }
    }

    true
}

fn read_collection_font(
    file: &mut Buffer<'_>,
    len: usize,
    fonts: &mut [Font],
    font_index: usize,
    all_tables: &mut BTreeMap<u32, (usize, u32)>,
) -> bool {
    match file.read_u32() {
        Some(v) => fonts[font_index].flavor = v,
        None => return false,
    }
    if !read_true_type_font(file, len, &mut fonts[font_index]) {
        return false;
    }

    let tags: Vec<u32> = fonts[font_index].tables.keys().copied().collect();
    for tag in tags {
        let offset = fonts[font_index].tables[&tag].offset;
        match all_tables.get(&offset) {
            None => {
                all_tables.insert(offset, (font_index, tag));
            }
            Some(&(orig_font, orig_tag)) => {
                if tag != orig_tag {
                    return false;
                }
                // Tags are unique within a font, so the original is always in
                // an earlier font.
                debug_assert_ne!(orig_font, font_index);
                fonts[font_index].tables.get_mut(&tag).unwrap().reuse_of = Some(orig_font);
            }
        }
    }
    true
}

fn read_true_type_collection(
    file: &mut Buffer<'_>,
    len: usize,
    font_collection: &mut FontCollection,
) -> bool {
    match file.read_u32() {
        Some(v) => font_collection.header_version = v,
        None => return false,
    }
    let num_fonts = match file.read_u32() {
        Some(v) => v,
        None => return false,
    };

    let mut offsets: Vec<u32> = Vec::new();
    for _ in 0..num_fonts {
        match file.read_u32() {
            Some(offset) => offsets.push(offset),
            None => return false,
        }
    }

    font_collection.fonts = vec![Font::default(); offsets.len()];

    let mut all_tables: BTreeMap<u32, (usize, u32)> = BTreeMap::new();
    for (i, &offset) in offsets.iter().enumerate() {
        if !file.set_offset(offset as usize) {
            return false;
        }
        if !read_collection_font(file, len, &mut font_collection.fonts, i, &mut all_tables) {
            return false;
        }
    }

    true
}

/// Parses the font (or font collection) from `data`.
pub fn read_font_collection(data: &[u8], font_collection: &mut FontCollection) -> bool {
    let len = data.len();
    let mut file = Buffer::new(data);

    match file.read_u32() {
        Some(v) => font_collection.flavor = v,
        None => return false,
    }

    if font_collection.flavor != TTC_FONT_FLAVOR {
        font_collection.fonts = vec![Font::default()];
        let font = &mut font_collection.fonts[0];
        font.flavor = font_collection.flavor;
        return read_true_type_font(&mut file, len, font);
    }
    read_true_type_collection(&mut file, len, font_collection)
}

/// Returns the number of glyphs in the font.
/// NOTE: Currently this works only for TrueType-flavored fonts, will return
/// zero for CFF-flavored fonts.
pub fn num_glyphs(font: &Font, input: &[u8]) -> i32 {
    let (head, loca) = match (
        font.find_table(HEAD_TABLE_TAG),
        font.find_table(LOCA_TABLE_TAG),
    ) {
        (Some(h), Some(l)) => (h, l),
        _ => return 0,
    };
    if head.length < 52 {
        return 0;
    }
    let index_fmt = index_format(font, input);
    let loca_record_size: u32 = if index_fmt == 0 { 2 } else { 4 };
    if loca.length < loca_record_size {
        return 0;
    }
    (loca.length / loca_record_size).wrapping_sub(1) as i32
}

/// Returns the index format of the font.
pub fn index_format(font: &Font, input: &[u8]) -> i32 {
    match font.find_table(HEAD_TABLE_TAG) {
        None => 0,
        Some(head) => i32::from(head.data(input).get(51).copied().unwrap_or(0)),
    }
}

/// Returns the data of the glyph with the given index.
pub fn get_glyph_data<'a>(font: &'a Font, glyph_index: i32, input: &'a [u8]) -> Option<&'a [u8]> {
    if glyph_index < 0 {
        return None;
    }
    let head = font.find_table(HEAD_TABLE_TAG)?;
    let loca = font.find_table(LOCA_TABLE_TAG)?;
    let glyf = font.find_table(GLYF_TABLE_TAG)?;
    if head.length < 52 {
        return None;
    }

    let index_fmt = index_format(font, input);

    let mut loca_buf = Buffer::new(loca.data(input));
    let (start, size) = if index_fmt == 0 {
        if !loca_buf.skip(2 * glyph_index as usize) {
            return None;
        }
        let offset1 = loca_buf.read_u16()?;
        let offset2 = loca_buf.read_u16()?;
        if offset2 < offset1 || 2 * u32::from(offset2) > glyf.length {
            return None;
        }
        (2 * usize::from(offset1), 2 * usize::from(offset2 - offset1))
    } else {
        if !loca_buf.skip(4 * glyph_index as usize) {
            return None;
        }
        let offset1 = loca_buf.read_u32()?;
        let offset2 = loca_buf.read_u32()?;
        if offset2 < offset1 || offset2 > glyf.length {
            return None;
        }
        (offset1 as usize, (offset2 - offset1) as usize)
    };
    let glyf_data = glyf.data(input);
    // C++ hands out a raw pointer here; only the in-bounds part can be read.
    let avail = &glyf_data[start.min(glyf_data.len())..];
    let slice = &avail[..size.min(avail.len())];
    if slice.len() != size {
        // The C++ code would read past the end of the glyf data here.
        return None;
    }
    Some(slice)
}

/// Removes the digital signature (DSIG) table.
pub fn remove_digital_signature(font: &mut Font) -> bool {
    if font.tables.remove(&DSIG_TABLE_TAG).is_some() {
        font.num_tables = font.tables.len() as u16;
    }
    true
}
