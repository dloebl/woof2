//! Converting TTF/OTF/TTC fonts to WOFF2 (`woff2_enc.cc`).

pub mod font;
pub mod glyph;
pub mod normalize;
pub mod transform;

use std::collections::BTreeMap;
use std::mem::MaybeUninit;

use crate::brotli::{self, CompressFn, BROTLI_MODE_FONT, BROTLI_MODE_TEXT};
use crate::common::{
    collection_header_size, TTC_FONT_FLAVOR, WOFF2_FLAGS_TRANSFORM, WOFF2_SIGNATURE,
};
use crate::store::{round4_u32, round4_usize};
use crate::table_tags::{GLYF_TABLE_TAG, KNOWN_TAGS, LOCA_TABLE_TAG};
use crate::variable_length::{base128_size, encode_255_ushort, encode_base128, size_255_ushort};

use font::{read_font_collection, resolve, Font, FontCollection};
use normalize::normalize_font_collection;
use transform::transform_glyf_and_loca_tables;

const WOFF2_HEADER_SIZE: usize = 48;

/// Parameters for the conversion, equivalent to `woff2::WOFF2Params`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Woff2Params {
    pub extended_metadata: Vec<u8>,
    pub brotli_quality: i32,
    pub allow_transforms: bool,
}

impl Default for Woff2Params {
    fn default() -> Self {
        Woff2Params {
            extended_metadata: Vec::new(),
            brotli_quality: 11,
            allow_transforms: true,
        }
    }
}

/// A table entry of the WOFF2 table directory.
#[derive(Clone, Copy, Debug, Default)]
struct Table {
    tag: u32,
    flags: u32,
    src_length: u32,
    transform_length: u32,
}

fn known_table_index(tag: u32) -> u8 {
    KNOWN_TAGS
        .iter()
        .position(|&t| t == tag)
        .map(|i| i as u8)
        .unwrap_or(63)
}

/// Sequential writer into the caller's result buffer.
///
/// The C++ code writes without bounds checks and only compares the final
/// offset with the precomputed length at the very end. Writes that would land
/// outside of the caller's buffer are dropped here instead; such a write
/// implies a length mismatch, so the conversion fails exactly like in C++.
struct Writer<'a> {
    buf: &'a mut [MaybeUninit<u8>],
    offset: usize,
}

impl Writer<'_> {
    #[inline]
    fn byte(&mut self, b: u8) {
        if let Some(slot) = self.buf.get_mut(self.offset) {
            slot.write(b);
        }
        self.offset = self.offset.wrapping_add(1);
    }

    fn bytes(&mut self, data: &[u8]) {
        let start = self.offset.min(self.buf.len());
        let n = data.len().min(self.buf.len() - start);
        for (slot, &b) in self.buf[start..start + n].iter_mut().zip(data) {
            slot.write(b);
        }
        self.offset = self.offset.wrapping_add(data.len());
    }

    fn u32(&mut self, v: u32) {
        self.bytes(&v.to_be_bytes());
    }

    fn u16(&mut self, v: i32) {
        self.byte((v >> 8) as u8);
        self.byte(v as u8);
    }

    fn ushort_255(&mut self, v: i32) {
        let mut tmp = [0u8; 3];
        let n = encode_255_ushort(v, &mut tmp);
        self.bytes(&tmp[..n]);
    }

    fn base128(&mut self, v: usize) {
        let mut tmp = [0u8; 10];
        let n = encode_base128(v, &mut tmp);
        self.bytes(&tmp[..n]);
    }
}

fn store_table_entry(table: &Table, w: &mut Writer<'_>) {
    let flag_byte = (table.flags & 0xC0) as u8 | known_table_index(table.tag);
    w.byte(flag_byte);
    // The index here is treated as a set of flag bytes because
    // bits 6 and 7 of the byte are reserved for future use as flags.
    // 0x3f or 63 means an arbitrary table tag.
    if (flag_byte & 0x3f) == 0x3f {
        w.u32(table.tag);
    }
    w.base128(table.src_length as usize);
    if (table.flags & WOFF2_FLAGS_TRANSFORM) != 0 {
        w.base128(table.transform_length as usize);
    }
}

fn table_entry_size(table: &Table) -> usize {
    let flag_byte = known_table_index(table.tag);
    let mut size = if (flag_byte & 0x3f) != 0x3f { 1 } else { 5 };
    size += base128_size(table.src_length as usize);
    if (table.flags & WOFF2_FLAGS_TRANSFORM) != 0 {
        size += base128_size(table.transform_length as usize);
    }
    size
}

fn compute_woff2_length(
    font_collection: &FontCollection,
    tables: &[Table],
    index_by_tag_offset: &BTreeMap<(u32, u32), u16>,
    compressed_data_length: usize,
    extended_metadata_length: usize,
) -> usize {
    let mut size = WOFF2_HEADER_SIZE;

    for table in tables {
        size = size.wrapping_add(table_entry_size(table));
    }

    // for collections only, collection tables
    if font_collection.flavor == TTC_FONT_FLAVOR {
        size = size.wrapping_add(4); // UInt32 Version of TTC Header
        size = size.wrapping_add(size_255_ushort(font_collection.fonts.len() as u16)); // numFonts

        size = size.wrapping_add(4usize.wrapping_mul(font_collection.fonts.len())); // flavors

        for font in &font_collection.fonts {
            size = size.wrapping_add(size_255_ushort(font.tables.len() as u16)); // numTables
            for table in font.tables.values() {
                // no collection entry for xform table
                if table.tag & 0x8080_8080 != 0 {
                    continue;
                }

                // (The C++ code uses operator[] on a copy of the map here,
                // so a missing entry counts as index 0.)
                let table_index = index_by_tag_offset
                    .get(&(table.tag, table.offset))
                    .copied()
                    .unwrap_or(0);
                size = size.wrapping_add(size_255_ushort(table_index)); // index entry
            }
        }
    }

    // compressed data
    size = size.wrapping_add(compressed_data_length);
    size = round4_usize(size);

    size.wrapping_add(extended_metadata_length)
}

fn compute_uncompressed_length_font(font: &Font) -> usize {
    // sfnt header + offset table
    let mut size: usize = 12 + 16 * usize::from(font.num_tables);
    for table in font.tables.values() {
        if table.tag & 0x8080_8080 != 0 {
            continue; // xform tables don't stay
        }
        if table.is_reused() {
            continue; // don't have to pay twice
        }
        size = size.wrapping_add(round4_u32(table.length) as usize);
    }
    size
}

fn compute_uncompressed_length(font_collection: &FontCollection) -> usize {
    if font_collection.flavor != TTC_FONT_FLAVOR {
        return compute_uncompressed_length_font(&font_collection.fonts[0]);
    }
    let mut size = collection_header_size(
        font_collection.header_version,
        font_collection.fonts.len() as u32,
    );
    for font in &font_collection.fonts {
        size = size.wrapping_add(compute_uncompressed_length_font(font));
    }
    size
}

fn compute_total_transform_length(font: &Font) -> usize {
    let mut total: usize = 0;
    for table in font.tables.values() {
        if table.is_reused() {
            continue;
        }
        if table.tag & 0x8080_8080 != 0 || font.find_table(table.tag ^ 0x8080_8080).is_none() {
            // Count transformed tables and non-transformed tables that do not
            // have transformed versions.
            total = total.wrapping_add(table.length as usize);
        }
    }
    total
}

/// Returns an upper bound on the size of the compressed file, for an input
/// font of `length` bytes.
pub fn max_woff2_compressed_size(length: usize, extended_metadata_length: usize) -> usize {
    // Except for the header size, which is 32 bytes larger in woff2 format,
    // all other parts should be smaller (table header in short format,
    // transformations and compression). Just to be sure, we will give some
    // headroom anyway.
    length
        .wrapping_add(1024)
        .wrapping_add(extended_metadata_length)
}

/// Size of the buffer handed to brotli for `original_size` bytes of input.
pub fn compressed_buffer_size(original_size: u32) -> u32 {
    (1.2 * f64::from(original_size) + 10240.0) as u32
}

fn transform_font_collection(font_collection: &mut FontCollection, input: &[u8]) -> bool {
    for font in &mut font_collection.fonts {
        if !transform_glyf_and_loca_tables(font, input) {
            noisy!("glyf/loca transformation failed.\n");
            return false;
        }
    }
    true
}

fn try_zeroed(len: usize) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    v.try_reserve_exact(len).ok()?;
    v.resize(len, 0);
    Some(v)
}

/// Compresses the font into `result`, using `compress` (which must behave
/// like `BrotliEncoderCompress`).
///
/// On entry `*result_length` is the number of usable bytes in `result` (it
/// should be at least [`max_woff2_compressed_size`]); on success it is set to
/// the actual compressed size. Like the C++ implementation, bytes of `result`
/// that are not part of the WOFF2 data (the up to three padding bytes before
/// the extended metadata or at the end) are left untouched.
pub fn convert_ttf_to_woff2_into(
    data: &[u8],
    result: &mut [MaybeUninit<u8>],
    result_length: &mut usize,
    params: &Woff2Params,
    compress: CompressFn,
) -> bool {
    let capacity = (*result_length).min(result.len());
    let result = &mut result[..capacity];

    let mut font_collection = FontCollection::default();
    if !read_font_collection(data, &mut font_collection) {
        noisy!("Parsing of the input font failed.\n");
        return false;
    }

    if !normalize_font_collection(&mut font_collection, data) {
        return false;
    }

    if params.allow_transforms && !transform_font_collection(&mut font_collection, data) {
        return false;
    } else {
        // glyf/loca use 11 to flag "not transformed"
        for font in &mut font_collection.fonts {
            if let Some(glyf) = font.find_table_mut(GLYF_TABLE_TAG) {
                glyf.flag_byte |= 0xc0;
            }
            if let Some(loca) = font.find_table_mut(LOCA_TABLE_TAG) {
                loca.flag_byte |= 0xc0;
            }
        }
    }

    // Although the compressed size of each table in the final woff2 file
    // won't be larger than its transform_length, we have to allocate a large
    // enough buffer for the compressor, since the compressor can potentially
    // increase the size. If the compressor overflows this, it should return
    // false and then this function will also return false.

    let mut total_transform_length: usize = 0;
    for font in &font_collection.fonts {
        total_transform_length =
            total_transform_length.wrapping_add(compute_total_transform_length(font));
    }
    let compression_buffer_size = compressed_buffer_size(total_transform_length as u32);
    let mut compression_buf = match try_zeroed(compression_buffer_size as usize) {
        Some(v) => v,
        None => return false,
    };

    // Collect all transformed data into one place in output order.
    let mut transform_buf = match try_zeroed(total_transform_length) {
        Some(v) => v,
        None => return false,
    };
    let mut transform_offset: usize = 0;
    for font in &font_collection.fonts {
        for tag in font.output_ordered_tags() {
            let original = &font.tables[&tag];
            if original.is_reused() {
                continue;
            }
            if tag & 0x8080_8080 != 0 {
                continue;
            }
            let table_to_store = font.find_table(tag ^ 0x8080_8080).unwrap_or(original);

            let bytes = table_to_store.data(data);
            // NOTE: the C++ code overflows its buffer here (undefined
            // behavior) for some malformed collections, where a reused table
            // is stored but not accounted for in total_transform_length.
            if bytes.len() != table_to_store.length as usize
                || transform_offset + bytes.len() > transform_buf.len()
            {
                return false;
            }
            transform_buf[transform_offset..transform_offset + bytes.len()].copy_from_slice(bytes);
            transform_offset += bytes.len();
        }
    }

    // Compress all transformed data in one stream.
    let total_compressed_length = match brotli::compress(
        compress,
        params.brotli_quality,
        BROTLI_MODE_FONT,
        &transform_buf,
        &mut compression_buf,
    ) {
        Some(n) => n as u32,
        None => {
            noisy!("Compression of combined table failed.\n");
            return false;
        }
    };

    noisy!(
        "Compressed {} to {}.\n",
        total_transform_length,
        total_compressed_length
    );

    // Compress the extended metadata
    // TODO(user): how does this apply to collections
    let metadata_len = params.extended_metadata.len();
    let mut compressed_metadata_buf_length = compressed_buffer_size(metadata_len as u32);
    let mut compressed_metadata_buf = match try_zeroed(compressed_metadata_buf_length as usize) {
        Some(v) => v,
        None => return false,
    };

    if metadata_len > 0 {
        match brotli::compress(
            compress,
            params.brotli_quality,
            BROTLI_MODE_TEXT,
            &params.extended_metadata,
            &mut compressed_metadata_buf,
        ) {
            Some(n) => compressed_metadata_buf_length = n as u32,
            None => {
                noisy!("Compression of extended metadata failed.\n");
                return false;
            }
        }
    } else {
        compressed_metadata_buf_length = 0;
    }

    let mut tables: Vec<Table> = Vec::new();
    let mut index_by_tag_offset: BTreeMap<(u32, u32), u16> = BTreeMap::new();

    for font in &font_collection.fonts {
        for tag in font.output_ordered_tags() {
            let src_table = &font.tables[&tag];
            if src_table.is_reused() {
                continue;
            }

            let tag_offset = (src_table.tag, src_table.offset);
            if index_by_tag_offset.contains_key(&tag_offset) {
                return false;
            }
            index_by_tag_offset.insert(tag_offset, tables.len() as u16);

            let mut table = Table {
                tag: src_table.tag,
                flags: u32::from(src_table.flag_byte),
                src_length: src_table.length,
                transform_length: src_table.length,
            };
            if let Some(transformed_table) = font.find_table(src_table.tag ^ 0x8080_8080) {
                table.flags = u32::from(transformed_table.flag_byte);
                table.flags |= WOFF2_FLAGS_TRANSFORM;
                table.transform_length = transformed_table.length;
            }
            tables.push(table);
        }
    }

    let woff2_length = compute_woff2_length(
        &font_collection,
        &tables,
        &index_by_tag_offset,
        total_compressed_length as usize,
        compressed_metadata_buf_length as usize,
    );
    if woff2_length > *result_length {
        noisy!(
            "Result allocation was too small ({} vs {} bytes).\n",
            *result_length as isize,
            woff2_length as isize
        );
        return false;
    }
    *result_length = woff2_length;

    let mut w = Writer {
        buf: result,
        offset: 0,
    };

    // start of woff2 header (http://www.w3.org/TR/WOFF2/#woff20Header)
    w.u32(WOFF2_SIGNATURE);
    if font_collection.flavor != TTC_FONT_FLAVOR {
        w.u32(font_collection.fonts[0].flavor);
    } else {
        w.u32(TTC_FONT_FLAVOR);
    }
    w.u32(woff2_length as u32);
    w.u16(tables.len() as i32);
    w.u16(0); // reserved
              // totalSfntSize
    w.u32(compute_uncompressed_length(&font_collection) as u32);
    w.u32(total_compressed_length); // totalCompressedSize

    // Let's just all be v1.0
    w.u16(1); // majorVersion
    w.u16(0); // minorVersion
    if compressed_metadata_buf_length > 0 {
        w.u32((woff2_length as u32).wrapping_sub(compressed_metadata_buf_length)); // metaOffset
        w.u32(compressed_metadata_buf_length); // metaLength
        w.u32(metadata_len as u32); // metaOrigLength
    } else {
        w.u32(0); // metaOffset
        w.u32(0); // metaLength
        w.u32(0); // metaOrigLength
    }
    w.u32(0); // privOffset
    w.u32(0); // privLength
              // end of woff2 header

    // table directory (http://www.w3.org/TR/WOFF2/#table_dir_format)
    for table in &tables {
        store_table_entry(table, &mut w);
    }

    // for collections only, collection table directory
    if font_collection.flavor == TTC_FONT_FLAVOR {
        w.u32(font_collection.header_version);
        w.ushort_255(font_collection.fonts.len() as i32);
        for (font_index, font) in font_collection.fonts.iter().enumerate() {
            let mut num_tables: u16 = 0;
            for table in font.tables.values() {
                if table.tag & 0x8080_8080 != 0 {
                    continue; // don't write xform tables
                }
                num_tables = num_tables.wrapping_add(1);
            }
            w.ushort_255(i32::from(num_tables));

            w.u32(font.flavor);
            for table in font.tables.values() {
                if table.tag & 0x8080_8080 != 0 {
                    continue; // don't write xform tables
                }

                // for reused tables, only the original has an updated offset
                let table_offset = if table.is_reused() {
                    let orig = resolve(&font_collection.fonts, font_index, table.tag);
                    font_collection.fonts[orig]
                        .tables
                        .get(&table.tag)
                        .map(|t| t.offset)
                        .unwrap_or(table.offset)
                } else {
                    table.offset
                };
                let tag_offset = (table.tag, table_offset);
                match index_by_tag_offset.get(&tag_offset) {
                    None => {
                        noisy!("Missing table index for offset 0x{:08x}\n", table_offset);
                        return false;
                    }
                    Some(&index) => w.ushort_255(i32::from(index)),
                }
            }
        }
    }

    // compressed data format (http://www.w3.org/TR/WOFF2/#table_format)

    w.bytes(&compression_buf[..total_compressed_length as usize]);
    w.offset = round4_usize(w.offset);

    w.bytes(&compressed_metadata_buf[..compressed_metadata_buf_length as usize]);

    if *result_length != w.offset {
        noisy!(
            "Mismatch between computed and actual length ({} vs {})\n",
            *result_length as isize,
            w.offset as isize
        );
        return false;
    }
    true
}

/// Compresses the font into `result`, using `compress` (which must behave
/// like `BrotliEncoderCompress`). See [`convert_ttf_to_woff2_into`].
pub fn convert_ttf_to_woff2_with(
    data: &[u8],
    result: &mut [u8],
    result_length: &mut usize,
    params: &Woff2Params,
    compress: CompressFn,
) -> bool {
    // SAFETY: `u8` and `MaybeUninit<u8>` have the same layout, and the
    // callee only ever writes initialized bytes.
    let result = unsafe {
        std::slice::from_raw_parts_mut(result.as_mut_ptr().cast::<MaybeUninit<u8>>(), result.len())
    };
    convert_ttf_to_woff2_into(data, result, result_length, params, compress)
}

/// Compresses the font using the system brotli library. Returns the WOFF2
/// data on success.
#[cfg(feature = "system-brotli")]
pub fn convert_ttf_to_woff2(data: &[u8], params: &Woff2Params) -> Option<Vec<u8>> {
    let mut result_length = max_woff2_compressed_size(data.len(), params.extended_metadata.len());
    let mut result = vec![0u8; result_length];
    if convert_ttf_to_woff2_with(
        data,
        &mut result,
        &mut result_length,
        params,
        brotli::SYSTEM_COMPRESS,
    ) {
        result.truncate(result_length);
        Some(result)
    } else {
        None
    }
}
