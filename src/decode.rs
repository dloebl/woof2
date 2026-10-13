//! Converting WOFF2 format font files to their TTF versions (`woff2_dec.cc`).
//!
//! This is a faithful port of the reference decoder. All integer widths and
//! wrap-around semantics of the C++ code are preserved, as is the exact
//! sequence of calls made on the [`Woff2Out`]: the decoder writes the very
//! same bytes in the very same order, so any custom output implementation
//! observes identical behavior.

use std::collections::BTreeMap;

use crate::brotli::{self, DecompressFn};
use crate::buffer::Buffer;
use crate::common::{
    collection_header_size, compute_ulong_sum, SFNT_ENTRY_SIZE, SFNT_HEADER_SIZE, TTC_FONT_FLAVOR,
    WOFF2_FLAGS_TRANSFORM, WOFF2_SIGNATURE,
};
use crate::output::Woff2Out;
use crate::store::{round4_u32, round4_u64, round4_usize, store16, store_u32};
use crate::table_tags::{
    GLYF_TABLE_TAG, HEAD_TABLE_TAG, HHEA_TABLE_TAG, HMTX_TABLE_TAG, KNOWN_TAGS, LOCA_TABLE_TAG,
};
use crate::variable_length::{read_255_ushort, read_base128};

// simple glyph flags
const GLYF_ON_CURVE: i32 = 1 << 0;
const GLYF_X_SHORT: i32 = 1 << 1;
const GLYF_Y_SHORT: i32 = 1 << 2;
const GLYF_REPEAT: u8 = 1 << 3;
const GLYF_THIS_X_IS_SAME: i32 = 1 << 4;
const GLYF_THIS_Y_IS_SAME: i32 = 1 << 5;
const OVERLAP_SIMPLE: i32 = 1 << 6;

// composite glyph flags
const FLAG_ARG_1_AND_2_ARE_WORDS: u16 = 1 << 0;
const FLAG_WE_HAVE_A_SCALE: u16 = 1 << 3;
const FLAG_MORE_COMPONENTS: u16 = 1 << 5;
const FLAG_WE_HAVE_AN_X_AND_Y_SCALE: u16 = 1 << 6;
const FLAG_WE_HAVE_A_TWO_BY_TWO: u16 = 1 << 7;
const FLAG_WE_HAVE_INSTRUCTIONS: u16 = 1 << 8;

// glyf flags
const FLAG_OVERLAP_SIMPLE_BITMAP: u16 = 1 << 0;

const END_PTS_OF_CONTOURS_OFFSET: usize = 10;

// 98% of Google Fonts have no glyph above 5k bytes
// Largest glyph ever observed was 72k bytes
const DEFAULT_GLYPH_BUF: usize = 5120;

// Over 14k test fonts the max compression ratio seen to date was ~20.
// >100 suggests you wrote a bad uncompressed size.
const MAX_PLAUSIBLE_COMPRESSION_RATIO: f32 = 100.0;

#[derive(Clone, Copy, Debug, Default)]
struct Table {
    tag: u32,
    flags: u32,
    src_offset: u32,
    src_length: u32,
    transform_length: u32,
    dst_offset: u32,
    dst_length: u32,
}

/// Metadata for a TTC font entry.
#[derive(Clone, Debug, Default)]
struct TtcFont {
    flavor: u32,
    dst_offset: u32,
    header_checksum: u32,
    table_indices: Vec<u16>,
}

#[derive(Debug, Default)]
struct Woff2Header {
    flavor: u32,
    header_version: u32,
    num_tables: u16,
    compressed_offset: u64,
    compressed_length: u32,
    uncompressed_size: u32,
    tables: Vec<Table>,
    ttc_fonts: Vec<TtcFont>,
}

/// Accumulates data we may need to reconstruct a single font. One per font
/// created for a TTC.
#[derive(Debug, Default)]
struct Woff2FontInfo {
    num_glyphs: u16,
    index_format: u16,
    num_hmetrics: u16,
    x_mins: Vec<i16>,
    table_entry_by_tag: BTreeMap<u32, u32>,
}

/// Accumulates metadata as we rebuild the font.
#[derive(Debug, Default)]
struct RebuildMetadata {
    header_checksum: u32,
    font_infos: Vec<Woff2FontInfo>,
    /// Checksums for tables that have been written.
    /// (tag, src_offset) => checksum. Need both because 0-length loca.
    checksums: BTreeMap<(u32, u32), u32>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Point {
    x: i32,
    y: i32,
    on_curve: bool,
}

#[inline]
fn with_sign(flag: i32, baseval: i32) -> i32 {
    // Precondition: 0 <= baseval < 65536 (to avoid integer overflow)
    if flag & 1 != 0 {
        baseval
    } else {
        -baseval
    }
}

fn triplet_decode(
    flags_in: &[u8],
    input: &[u8],
    n_points: u32,
    result: &mut [Point],
) -> Option<usize> {
    let in_size = input.len();
    let mut x: i32 = 0;
    let mut y: i32 = 0;

    if n_points as usize > in_size {
        return None;
    }
    let mut triplet_index: u32 = 0;

    for i in 0..n_points as usize {
        let mut flag = flags_in[i];
        let on_curve = (flag >> 7) == 0;
        flag &= 0x7f;
        let n_data_bytes: u32 = if flag < 84 {
            1
        } else if flag < 120 {
            2
        } else if flag < 124 {
            3
        } else {
            4
        };
        let end = triplet_index.wrapping_add(n_data_bytes);
        if end as usize > in_size || end < triplet_index {
            return None;
        }
        let t = triplet_index as usize;
        let flag_i = i32::from(flag);
        let b = |k: usize| i32::from(input[t + k]);
        let (dx, dy);
        if flag < 10 {
            dx = 0;
            dy = with_sign(flag_i, ((flag_i & 14) << 7) + b(0));
        } else if flag < 20 {
            dx = with_sign(flag_i, (((flag_i - 10) & 14) << 7) + b(0));
            dy = 0;
        } else if flag < 84 {
            let b0 = flag_i - 20;
            let b1 = b(0);
            dx = with_sign(flag_i, 1 + (b0 & 0x30) + (b1 >> 4));
            dy = with_sign(flag_i >> 1, 1 + ((b0 & 0x0c) << 2) + (b1 & 0x0f));
        } else if flag < 120 {
            let b0 = flag_i - 84;
            dx = with_sign(flag_i, 1 + ((b0 / 12) << 8) + b(0));
            dy = with_sign(flag_i >> 1, 1 + (((b0 % 12) >> 2) << 8) + b(1));
        } else if flag < 124 {
            let b2 = b(1);
            dx = with_sign(flag_i, (b(0) << 4) + (b2 >> 4));
            dy = with_sign(flag_i >> 1, ((b2 & 0x0f) << 8) + b(2));
        } else {
            dx = with_sign(flag_i, (b(0) << 8) + b(1));
            dy = with_sign(flag_i >> 1, (b(2) << 8) + b(3));
        }
        triplet_index = end;
        x = x.checked_add(dx)?;
        y = y.checked_add(dy)?;
        result[i] = Point { x, y, on_curve };
    }
    Some(triplet_index as usize)
}

/// Stores just the point data. On entry, `dst` holds the beginning of a
/// simple glyph. Returns the glyph size on success.
fn store_points(
    points: &[Point],
    n_contours: u32,
    instruction_length: u32,
    has_overlap_bit: bool,
    dst: &mut [u8],
) -> Option<usize> {
    let dst_size = dst.len();
    let mut flag_offset: u32 = (END_PTS_OF_CONTOURS_OFFSET as u32)
        .wrapping_add(2u32.wrapping_mul(n_contours))
        .wrapping_add(2)
        .wrapping_add(instruction_length);
    let mut last_flag: i32 = -1;
    let mut repeat_count: i32 = 0;
    let mut last_x: i32 = 0;
    let mut last_y: i32 = 0;
    let mut x_bytes: u32 = 0;
    let mut y_bytes: u32 = 0;

    for (i, point) in points.iter().enumerate() {
        let mut flag = if point.on_curve { GLYF_ON_CURVE } else { 0 };
        if has_overlap_bit && i == 0 {
            flag |= OVERLAP_SIMPLE;
        }

        let dx = point.x.wrapping_sub(last_x);
        let dy = point.y.wrapping_sub(last_y);
        if dx == 0 {
            flag |= GLYF_THIS_X_IS_SAME;
        } else if dx > -256 && dx < 256 {
            flag |= GLYF_X_SHORT | if dx > 0 { GLYF_THIS_X_IS_SAME } else { 0 };
            x_bytes = x_bytes.wrapping_add(1);
        } else {
            x_bytes = x_bytes.wrapping_add(2);
        }
        if dy == 0 {
            flag |= GLYF_THIS_Y_IS_SAME;
        } else if dy > -256 && dy < 256 {
            flag |= GLYF_Y_SHORT | if dy > 0 { GLYF_THIS_Y_IS_SAME } else { 0 };
            y_bytes = y_bytes.wrapping_add(1);
        } else {
            y_bytes = y_bytes.wrapping_add(2);
        }

        if flag == last_flag && repeat_count != 255 {
            dst[flag_offset as usize - 1] |= GLYF_REPEAT;
            repeat_count += 1;
        } else {
            if repeat_count != 0 {
                if flag_offset as usize >= dst_size {
                    return None;
                }
                dst[flag_offset as usize] = repeat_count as u8;
                flag_offset += 1;
            }
            if flag_offset as usize >= dst_size {
                return None;
            }
            dst[flag_offset as usize] = flag as u8;
            flag_offset += 1;
            repeat_count = 0;
        }
        last_x = point.x;
        last_y = point.y;
        last_flag = flag;
    }

    if repeat_count != 0 {
        if flag_offset as usize >= dst_size {
            return None;
        }
        dst[flag_offset as usize] = repeat_count as u8;
        flag_offset += 1;
    }
    let xy_bytes = x_bytes.wrapping_add(y_bytes);
    if xy_bytes < x_bytes
        || flag_offset.wrapping_add(xy_bytes) < flag_offset
        || flag_offset.wrapping_add(xy_bytes) as usize > dst_size
    {
        return None;
    }

    let mut x_offset = flag_offset as usize;
    let mut y_offset = (flag_offset + x_bytes) as usize;
    last_x = 0;
    last_y = 0;
    for point in points {
        let dx = point.x.wrapping_sub(last_x);
        if dx == 0 {
            // pass
        } else if dx > -256 && dx < 256 {
            dst[x_offset] = dx.unsigned_abs() as u8;
            x_offset += 1;
        } else {
            // will always fit for valid input, but overflow is harmless
            x_offset = store16(dst, x_offset, dx);
        }
        last_x = last_x.wrapping_add(dx);
        let dy = point.y.wrapping_sub(last_y);
        if dy == 0 {
            // pass
        } else if dy > -256 && dy < 256 {
            dst[y_offset] = dy.unsigned_abs() as u8;
            y_offset += 1;
        } else {
            y_offset = store16(dst, y_offset, dy);
        }
        last_y = last_y.wrapping_add(dy);
    }
    Some(y_offset)
}

/// Computes the bounding box of the coordinates, and stores it into a glyf
/// buffer. `dst` must have at least 10 bytes.
fn compute_bbox(points: &[Point], dst: &mut [u8]) {
    let mut x_min = 0;
    let mut y_min = 0;
    let mut x_max = 0;
    let mut y_max = 0;

    if let Some(p) = points.first() {
        x_min = p.x;
        x_max = p.x;
        y_min = p.y;
        y_max = p.y;
    }
    for p in points.iter().skip(1) {
        x_min = x_min.min(p.x);
        x_max = x_max.max(p.x);
        y_min = y_min.min(p.y);
        y_max = y_max.max(p.y);
    }
    let mut offset = 2;
    offset = store16(dst, offset, x_min);
    offset = store16(dst, offset, y_min);
    offset = store16(dst, offset, x_max);
    store16(dst, offset, y_max);
}

/// Returns (size, have_instructions) of the composite glyph data at the
/// current position of `composite_stream` (which is taken by value, so the
/// caller's position is not advanced).
fn size_of_composite(mut composite_stream: Buffer<'_>) -> Option<(usize, bool)> {
    let start_offset = composite_stream.offset();
    let mut we_have_instructions = false;

    let mut flags = FLAG_MORE_COMPONENTS;
    while flags & FLAG_MORE_COMPONENTS != 0 {
        flags = composite_stream.read_u16()?;
        we_have_instructions |= (flags & FLAG_WE_HAVE_INSTRUCTIONS) != 0;
        let mut arg_size: usize = 2; // glyph index
        if flags & FLAG_ARG_1_AND_2_ARE_WORDS != 0 {
            arg_size += 4;
        } else {
            arg_size += 2;
        }
        if flags & FLAG_WE_HAVE_A_SCALE != 0 {
            arg_size += 2;
        } else if flags & FLAG_WE_HAVE_AN_X_AND_Y_SCALE != 0 {
            arg_size += 4;
        } else if flags & FLAG_WE_HAVE_A_TWO_BY_TWO != 0 {
            arg_size += 8;
        }
        if !composite_stream.skip(arg_size) {
            return None;
        }
    }

    Some((
        composite_stream.offset() - start_offset,
        we_have_instructions,
    ))
}

fn pad4(out: &mut dyn Woff2Out) -> bool {
    const ZEROES: [u8; 3] = [0, 0, 0];
    let a = out.size();
    let b = out.size();
    if a.wrapping_add(3) < b {
        return false;
    }
    let a = out.size();
    let b = out.size();
    let pad_bytes = round4_usize(a).wrapping_sub(b) as u32;
    if pad_bytes > 0 {
        // Only reachable with a WOFF2Out whose Size() is not a pure function;
        // the C++ code would read past its 3-byte array of zeroes here.
        if pad_bytes as usize > ZEROES.len() {
            return false;
        }
        if !out.write(&ZEROES[..pad_bytes as usize]) {
            return false;
        }
    }
    true
}

/// Builds the TrueType loca table.
fn store_loca(
    loca_values: &[u32],
    index_format: i32,
    checksum: &mut u32,
    out: &mut dyn Woff2Out,
) -> bool {
    let loca_size = loca_values.len() as u64;
    let offset_size: u64 = if index_format != 0 { 4 } else { 2 };
    if (loca_size << 2) >> 2 != loca_size {
        return false;
    }
    let mut loca_content = Vec::new();
    if loca_content
        .try_reserve_exact((loca_size * offset_size) as usize)
        .is_err()
    {
        return false;
    }
    for &value in loca_values {
        if index_format != 0 {
            loca_content.extend_from_slice(&value.to_be_bytes());
        } else {
            loca_content.extend_from_slice(&((value >> 1) as u16).to_be_bytes());
        }
    }
    *checksum = compute_ulong_sum(&loca_content);
    out.write(&loca_content)
}

/// Copies `n` bytes from `stream` into `dst[at..at + n]`.
#[inline]
fn read_into(stream: &mut Buffer<'_>, dst: &mut [u8], at: usize, n: usize) -> bool {
    match stream.read(n) {
        Some(src) => {
            dst[at..at + n].copy_from_slice(src);
            true
        }
        None => false,
    }
}

/// Reconstructs the entire glyf table based on the transformed original.
#[allow(clippy::too_many_arguments)]
fn reconstruct_glyf(
    data: &[u8],
    tables: &mut [Table],
    glyf_idx: usize,
    glyf_checksum: &mut u32,
    loca_idx: usize,
    loca_checksum: &mut u32,
    info: &mut Woff2FontInfo,
    out: &mut dyn Woff2Out,
) -> bool {
    const NUM_SUB_STREAMS: usize = 7;
    let transform_length = tables[glyf_idx].transform_length;
    let data = &data[..transform_length as usize];
    let mut file = Buffer::new(data);
    let glyf_start = out.size();

    if file.read_u16().is_none() {
        return false;
    }

    let flags = match file.read_u16() {
        Some(f) => f,
        None => return false,
    };
    let has_overlap_bitmap = flags & FLAG_OVERLAP_SIMPLE_BITMAP != 0;

    match file.read_u16() {
        Some(v) => info.num_glyphs = v,
        None => return false,
    }
    match file.read_u16() {
        Some(v) => info.index_format = v,
        None => return false,
    }

    // https://dev.w3.org/webfonts/WOFF2/spec/#conform-mustRejectLoca
    // dst_length here is origLength in the spec
    let expected_loca_dst_length: u32 =
        (if info.index_format != 0 { 4 } else { 2 }) * (u32::from(info.num_glyphs) + 1);
    if tables[loca_idx].dst_length != expected_loca_dst_length {
        return false;
    }

    let mut offset: u32 = (2 + NUM_SUB_STREAMS as u32) * 4;
    if offset > transform_length {
        return false;
    }
    // Invariant from here on: data_size >= offset
    let mut substreams = [(0usize, 0usize); NUM_SUB_STREAMS];
    for substream in substreams.iter_mut() {
        let substream_size = match file.read_u32() {
            Some(v) => v,
            None => return false,
        };
        if substream_size > transform_length - offset {
            return false;
        }
        *substream = (offset as usize, substream_size as usize);
        offset += substream_size;
    }
    let sub = |i: usize| Buffer::new(&data[substreams[i].0..substreams[i].0 + substreams[i].1]);
    let mut n_contour_stream = sub(0);
    let mut n_points_stream = sub(1);
    let mut flag_stream = sub(2);
    let mut glyph_stream = sub(3);
    let mut composite_stream = sub(4);
    let mut bbox_stream = sub(5);
    let mut instruction_stream = sub(6);

    let mut overlap_bitmap: &[u8] = &[];
    if has_overlap_bitmap {
        let overlap_bitmap_length = (u32::from(info.num_glyphs) + 7) >> 3;
        if overlap_bitmap_length > transform_length - offset {
            return false;
        }
        overlap_bitmap = &data[offset as usize..offset as usize + overlap_bitmap_length as usize];
    }

    let num_glyphs = u32::from(info.num_glyphs);
    let mut loca_values = vec![0u32; num_glyphs as usize + 1];
    let mut n_points_vec: Vec<u32> = Vec::new();
    let mut points: Vec<Point> = Vec::new();
    let bbox_bitmap = bbox_stream.buffer();
    // Safe because num_glyphs is bounded
    let bitmap_length = (((num_glyphs + 31) >> 5) << 2) as usize;
    if !bbox_stream.skip(bitmap_length) {
        return false;
    }

    // Temp buffer for glyphs.
    let mut glyph_buf: Vec<u8> = vec![0u8; DEFAULT_GLYPH_BUF];

    info.x_mins.resize(num_glyphs as usize, 0);
    for i in 0..num_glyphs {
        let iu = i as usize;
        let mut glyph_size: usize = 0;
        let have_bbox = bbox_bitmap[iu >> 3] & (0x80 >> (i & 7)) != 0;
        let n_contours = match n_contour_stream.read_u16() {
            Some(v) => v,
            None => return false,
        };

        if n_contours == 0xffff {
            // composite glyph
            let mut instruction_size: u32 = 0;
            if !have_bbox {
                // composite glyphs must have an explicit bbox
                return false;
            }

            let (composite_size, have_instructions) = match size_of_composite(composite_stream) {
                Some(v) => v,
                None => return false,
            };
            if have_instructions {
                match read_255_ushort(&mut glyph_stream) {
                    Some(v) => instruction_size = v,
                    None => return false,
                }
            }

            let size_needed = 12 + composite_size + instruction_size as usize;
            if glyph_buf.len() < size_needed {
                glyph_buf = vec![0u8; size_needed];
            }

            glyph_size = store16(&mut glyph_buf, glyph_size, i32::from(n_contours));
            if !read_into(&mut bbox_stream, &mut glyph_buf, glyph_size, 8) {
                return false;
            }
            glyph_size += 8;

            if !read_into(
                &mut composite_stream,
                &mut glyph_buf,
                glyph_size,
                composite_size,
            ) {
                return false;
            }
            glyph_size += composite_size;
            if have_instructions {
                glyph_size = store16(&mut glyph_buf, glyph_size, instruction_size as i32);
                if !read_into(
                    &mut instruction_stream,
                    &mut glyph_buf,
                    glyph_size,
                    instruction_size as usize,
                ) {
                    return false;
                }
                glyph_size += instruction_size as usize;
            }
        } else if n_contours > 0 {
            // simple glyph
            n_points_vec.clear();
            let mut total_n_points: u32 = 0;
            for _ in 0..n_contours {
                let n_points_contour = match read_255_ushort(&mut n_points_stream) {
                    Some(v) => v,
                    None => return false,
                };
                n_points_vec.push(n_points_contour);
                if total_n_points.wrapping_add(n_points_contour) < total_n_points {
                    return false;
                }
                total_n_points += n_points_contour;
            }
            let flag_size = total_n_points;
            if flag_size as usize > flag_stream.len() - flag_stream.offset() {
                return false;
            }
            let flags_buf = flag_stream.remaining();
            let triplet_buf = glyph_stream.remaining();
            if points.len() < total_n_points as usize {
                points = vec![Point::default(); total_n_points as usize];
            }
            let triplet_bytes_consumed =
                match triplet_decode(flags_buf, triplet_buf, total_n_points, &mut points) {
                    Some(v) => v,
                    None => return false,
                };
            if !flag_stream.skip(flag_size as usize) {
                return false;
            }
            if !glyph_stream.skip(triplet_bytes_consumed) {
                return false;
            }
            let instruction_size = match read_255_ushort(&mut glyph_stream) {
                Some(v) => v,
                None => return false,
            };

            if total_n_points >= (1 << 27) || instruction_size >= (1 << 30) {
                return false;
            }
            let size_needed = 12usize
                + 2 * usize::from(n_contours)
                + 5 * total_n_points as usize
                + instruction_size as usize;
            if glyph_buf.len() < size_needed {
                glyph_buf = vec![0u8; size_needed];
            }

            glyph_size = store16(&mut glyph_buf, glyph_size, i32::from(n_contours));
            let points = &points[..total_n_points as usize];
            if have_bbox {
                if !read_into(&mut bbox_stream, &mut glyph_buf, glyph_size, 8) {
                    return false;
                }
            } else {
                compute_bbox(points, &mut glyph_buf);
            }
            glyph_size = END_PTS_OF_CONTOURS_OFFSET;
            let mut end_point: i32 = -1;
            for &n in &n_points_vec {
                end_point = (end_point as u32).wrapping_add(n) as i32;
                if end_point >= 65536 {
                    return false;
                }
                glyph_size = store16(&mut glyph_buf, glyph_size, end_point);
            }

            glyph_size = store16(&mut glyph_buf, glyph_size, instruction_size as i32);
            if !read_into(
                &mut instruction_stream,
                &mut glyph_buf,
                glyph_size,
                instruction_size as usize,
            ) {
                return false;
            }
            // store_points derives the flag offset (== glyph_size here) itself.
            debug_assert_eq!(
                glyph_size + instruction_size as usize,
                END_PTS_OF_CONTOURS_OFFSET
                    + 2 * usize::from(n_contours)
                    + 2
                    + instruction_size as usize
            );

            let has_overlap_bit =
                has_overlap_bitmap && overlap_bitmap[iu >> 3] & (0x80 >> (i & 7)) != 0;

            match store_points(
                points,
                u32::from(n_contours),
                instruction_size,
                has_overlap_bit,
                &mut glyph_buf,
            ) {
                Some(v) => glyph_size = v,
                None => return false,
            }
        } else {
            // n_contours == 0; empty glyph. Must NOT have a bbox.
            if have_bbox {
                noisy!("Empty glyph has a bbox\n");
                return false;
            }
        }

        loca_values[iu] = out.size().wrapping_sub(glyf_start) as u32;
        if !out.write(&glyph_buf[..glyph_size]) {
            return false;
        }

        // TODO(user) Old code aligned glyphs ... but do we actually need to?
        if !pad4(out) {
            return false;
        }

        *glyf_checksum = glyf_checksum.wrapping_add(compute_ulong_sum(&glyph_buf[..glyph_size]));

        // We may need x_min to reconstruct 'hmtx'
        if n_contours > 0 {
            info.x_mins[iu] = i16::from_be_bytes([glyph_buf[2], glyph_buf[3]]);
        }
    }

    // glyf_table dst_offset was set by reconstruct_font
    let glyf_dst_length = (out.size() as u32).wrapping_sub(tables[glyf_idx].dst_offset);
    tables[glyf_idx].dst_length = glyf_dst_length;
    tables[loca_idx].dst_offset = out.size() as u32;
    // loca[n] will be equal the length of the glyph data ('glyf') table
    loca_values[num_glyphs as usize] = glyf_dst_length;
    if !store_loca(
        &loca_values,
        i32::from(info.index_format),
        loca_checksum,
        out,
    ) {
        return false;
    }
    tables[loca_idx].dst_length = (out.size() as u32).wrapping_sub(tables[loca_idx].dst_offset);

    true
}

/// Returns the index (into `hdr.tables`) of the first table with `tag`.
fn find_table(tables: &[usize], all: &[Table], tag: u32) -> Option<usize> {
    tables.iter().copied().find(|&i| all[i].tag == tag)
}

/// Gets numberOfHMetrics, https://www.microsoft.com/typography/otspec/hhea.htm
fn read_num_hmetrics(data: &[u8]) -> Option<u16> {
    // Skip 34 to reach 'hhea' numberOfHMetrics
    let mut buffer = Buffer::new(data);
    if !buffer.skip(34) {
        return None;
    }
    buffer.read_u16()
}

/// http://dev.w3.org/webfonts/WOFF2/spec/Overview.html#hmtx_table_format
fn reconstruct_transformed_hmtx(
    transformed_buf: &[u8],
    num_glyphs: u16,
    num_hmetrics: u16,
    x_mins: &[i16],
    checksum: &mut u32,
    out: &mut dyn Woff2Out,
) -> bool {
    let mut hmtx_buff_in = Buffer::new(transformed_buf);

    let hmtx_flags = match hmtx_buff_in.read_u8() {
        Some(v) => v,
        None => return false,
    };

    let has_proportional_lsbs = (hmtx_flags & 1) == 0;
    let has_monospace_lsbs = (hmtx_flags & 2) == 0;

    // Bits 2-7 are reserved and MUST be zero.
    if (hmtx_flags & 0xFC) != 0 {
        noisy!("Illegal hmtx flags; bits 2-7 must be 0\n");
        return false;
    }

    // you say you transformed but there is little evidence of it
    if has_proportional_lsbs && has_monospace_lsbs {
        return false;
    }

    debug_assert_eq!(x_mins.len(), usize::from(num_glyphs));
    let x_min = |i: u16| x_mins.get(usize::from(i)).copied().unwrap_or(0);

    // num_glyphs 0 is OK if there is no 'glyf' but cannot then xform 'hmtx'.
    if num_hmetrics > num_glyphs {
        return false;
    }

    // https://www.microsoft.com/typography/otspec/hmtx.htm
    // "...only one entry need be in the array, but that entry is required."
    if num_hmetrics < 1 {
        return false;
    }

    let mut advance_widths: Vec<u16> = Vec::with_capacity(usize::from(num_hmetrics));
    for _ in 0..num_hmetrics {
        match hmtx_buff_in.read_u16() {
            Some(v) => advance_widths.push(v),
            None => return false,
        }
    }

    let mut lsbs: Vec<i16> = Vec::with_capacity(usize::from(num_glyphs));
    for i in 0..num_hmetrics {
        let lsb = if has_proportional_lsbs {
            match hmtx_buff_in.read_s16() {
                Some(v) => v,
                None => return false,
            }
        } else {
            x_min(i)
        };
        lsbs.push(lsb);
    }

    for i in num_hmetrics..num_glyphs {
        let lsb = if has_monospace_lsbs {
            match hmtx_buff_in.read_s16() {
                Some(v) => v,
                None => return false,
            }
        } else {
            x_min(i)
        };
        lsbs.push(lsb);
    }

    // bake me a shiny new hmtx table
    let hmtx_output_size = 2 * usize::from(num_glyphs) + 2 * usize::from(num_hmetrics);
    let mut hmtx_table: Vec<u8> = Vec::with_capacity(hmtx_output_size);
    for i in 0..usize::from(num_glyphs) {
        if i < usize::from(num_hmetrics) {
            hmtx_table.extend_from_slice(&advance_widths[i].to_be_bytes());
        }
        hmtx_table.extend_from_slice(&lsbs[i].to_be_bytes());
    }

    *checksum = compute_ulong_sum(&hmtx_table);
    out.write(&hmtx_table)
}

fn read_table_directory(file: &mut Buffer<'_>, tables: &mut [Table]) -> bool {
    let mut src_offset: u32 = 0;
    for table in tables.iter_mut() {
        let flag_byte = match file.read_u8() {
            Some(v) => v,
            None => return false,
        };
        let tag = if (flag_byte & 0x3f) == 0x3f {
            match file.read_u32() {
                Some(v) => v,
                None => return false,
            }
        } else {
            KNOWN_TAGS[usize::from(flag_byte & 0x3f)]
        };
        let mut flags: u32 = 0;
        let xform_version = (flag_byte >> 6) & 0x03;

        // 0 means xform for glyph/loca, non-0 for others
        if tag == GLYF_TABLE_TAG || tag == LOCA_TABLE_TAG {
            if xform_version == 0 {
                flags |= WOFF2_FLAGS_TRANSFORM;
            }
        } else if xform_version != 0 {
            flags |= WOFF2_FLAGS_TRANSFORM;
        }
        flags |= u32::from(xform_version);

        let dst_length = match read_base128(file) {
            Some(v) => v,
            None => return false,
        };
        let mut transform_length = dst_length;
        if (flags & WOFF2_FLAGS_TRANSFORM) != 0 {
            match read_base128(file) {
                Some(v) => transform_length = v,
                None => return false,
            }
            if tag == LOCA_TABLE_TAG && transform_length != 0 {
                return false;
            }
        }
        if src_offset.wrapping_add(transform_length) < src_offset {
            return false;
        }
        table.src_offset = src_offset;
        table.src_length = transform_length;
        src_offset += transform_length;

        table.tag = tag;
        table.flags = flags;
        table.transform_length = transform_length;
        table.dst_length = dst_length;
    }
    true
}

/// Writes a single Offset Table entry.
fn store_offset_table(result: &mut [u8], offset: usize, flavor: u32, num_tables: u16) -> usize {
    let mut offset = store_u32(result, offset, flavor); // sfnt version
    offset = store16(result, offset, i32::from(num_tables)); // num_tables
    let mut max_pow2: u32 = 0;
    while 1u32 << (max_pow2 + 1) <= u32::from(num_tables) {
        max_pow2 += 1;
    }
    let output_search_range = ((1u32 << max_pow2) << 4) as u16;
    offset = store16(result, offset, i32::from(output_search_range)); // searchRange
    offset = store16(result, offset, max_pow2 as i32); // entrySelector
                                                       // rangeShift
    offset = store16(
        result,
        offset,
        (i32::from(num_tables) << 4) - i32::from(output_search_range),
    );
    offset
}

fn store_table_entry(result: &mut [u8], offset: usize, tag: u32) -> usize {
    let mut offset = store_u32(result, offset, tag);
    offset = store_u32(result, offset, 0);
    offset = store_u32(result, offset, 0);
    store_u32(result, offset, 0)
}

/// First table goes after all the headers, table directory, etc.
fn compute_offset_to_first_table(hdr: &Woff2Header) -> u64 {
    let mut offset = SFNT_HEADER_SIZE as u64 + SFNT_ENTRY_SIZE as u64 * u64::from(hdr.num_tables);
    if hdr.header_version != 0 {
        offset = collection_header_size(hdr.header_version, hdr.ttc_fonts.len() as u32) as u64
            + SFNT_HEADER_SIZE as u64 * hdr.ttc_fonts.len() as u64;
        for ttc_font in &hdr.ttc_fonts {
            offset += SFNT_ENTRY_SIZE as u64 * ttc_font.table_indices.len() as u64;
        }
    }
    offset
}

fn font_tables(hdr: &Woff2Header, font_index: usize) -> Vec<usize> {
    if hdr.header_version != 0 {
        hdr.ttc_fonts[font_index]
            .table_indices
            .iter()
            .map(|&i| usize::from(i))
            .collect()
    } else {
        (0..hdr.tables.len()).collect()
    }
}

/// Offset tables assumed to have been written in with 0's initially.
fn reconstruct_font(
    transformed_buf: &mut [u8],
    metadata: &mut RebuildMetadata,
    hdr: &mut Woff2Header,
    font_index: usize,
    out: &mut dyn Woff2Out,
) -> bool {
    let transformed_buf_size = transformed_buf.len() as u64;
    let mut dest_offset: usize = out.size();
    let info = &mut metadata.font_infos[font_index];
    let tables = font_tables(hdr, font_index);

    // 'glyf' without 'loca' doesn't make sense
    let glyf_table = find_table(&tables, &hdr.tables, GLYF_TABLE_TAG);
    let loca_table = find_table(&tables, &hdr.tables, LOCA_TABLE_TAG);
    if glyf_table.is_some() != loca_table.is_some() {
        noisy!("Cannot have just one of glyf/loca\n");
        return false;
    }

    if let (Some(g), Some(l)) = (glyf_table, loca_table) {
        if (hdr.tables[g].flags & WOFF2_FLAGS_TRANSFORM)
            != (hdr.tables[l].flags & WOFF2_FLAGS_TRANSFORM)
        {
            noisy!("Cannot transform just one of glyf/loca\n");
            return false;
        }
    }

    let mut font_checksum = metadata.header_checksum;
    if hdr.header_version != 0 {
        font_checksum = hdr.ttc_fonts[font_index].header_checksum;
    }

    let mut loca_checksum: u32 = 0;
    for &ti in &tables {
        let table = hdr.tables[ti];

        let checksum_key = (table.tag, table.src_offset);
        let reused = metadata.checksums.contains_key(&checksum_key);
        if font_index == 0 && reused {
            return false;
        }

        // TODO(user) a collection with optimized hmtx that reused glyf/loca
        // would fail. We don't optimize hmtx for collections yet.
        if u64::from(table.src_offset) + u64::from(table.src_length) > transformed_buf_size {
            return false;
        }
        let src_start = table.src_offset as usize;
        let src_end = src_start + table.src_length as usize;

        if table.tag == HHEA_TABLE_TAG {
            match read_num_hmetrics(&transformed_buf[src_start..src_end]) {
                Some(v) => info.num_hmetrics = v,
                None => return false,
            }
        }

        let mut checksum: u32 = 0;
        if !reused {
            if (table.flags & WOFF2_FLAGS_TRANSFORM) != WOFF2_FLAGS_TRANSFORM {
                if table.tag == HEAD_TABLE_TAG {
                    if table.src_length < 12 {
                        return false;
                    }
                    // checkSumAdjustment = 0
                    store_u32(&mut transformed_buf[src_start..], 8, 0);
                }
                hdr.tables[ti].dst_offset = dest_offset as u32;
                let src = &transformed_buf[src_start..src_end];
                checksum = compute_ulong_sum(src);
                if !out.write(src) {
                    return false;
                }
            } else if table.tag == GLYF_TABLE_TAG {
                hdr.tables[ti].dst_offset = dest_offset as u32;

                let loca_idx = match find_table(&tables, &hdr.tables, LOCA_TABLE_TAG) {
                    Some(l) => l,
                    None => return false,
                };
                if !reconstruct_glyf(
                    &transformed_buf[src_start..],
                    &mut hdr.tables,
                    ti,
                    &mut checksum,
                    loca_idx,
                    &mut loca_checksum,
                    info,
                    out,
                ) {
                    return false;
                }
            } else if table.tag == LOCA_TABLE_TAG {
                // All the work was done by reconstruct_glyf. We already know checksum.
                checksum = loca_checksum;
            } else if table.tag == HMTX_TABLE_TAG {
                hdr.tables[ti].dst_offset = dest_offset as u32;
                // Tables are sorted so all the info we need has been gathered.
                if !reconstruct_transformed_hmtx(
                    &transformed_buf[src_start..src_end],
                    info.num_glyphs,
                    info.num_hmetrics,
                    &info.x_mins,
                    &mut checksum,
                    out,
                ) {
                    return false;
                }
            } else {
                return false; // transform unknown
            }
            metadata.checksums.insert(checksum_key, checksum);
        } else {
            checksum = metadata.checksums[&checksum_key];
        }
        font_checksum = font_checksum.wrapping_add(checksum);

        // update the table entry with real values.
        let table = hdr.tables[ti];
        let mut table_entry = [0u8; 12];
        store_u32(&mut table_entry, 0, checksum);
        store_u32(&mut table_entry, 4, table.dst_offset);
        store_u32(&mut table_entry, 8, table.dst_length);
        let entry_offset = *info.table_entry_by_tag.entry(table.tag).or_insert(0);
        if !out.write_at(&table_entry, entry_offset.wrapping_add(4) as usize) {
            return false;
        }

        // We replaced 0's. Update overall checksum.
        font_checksum = font_checksum.wrapping_add(compute_ulong_sum(&table_entry));

        if !pad4(out) {
            return false;
        }

        if u64::from(table.dst_offset.wrapping_add(table.dst_length)) > out.size() as u64 {
            return false;
        }
        dest_offset = out.size();
    }

    // Update 'head' checkSumAdjustment. We already set it to 0 and summed font.
    if let Some(head) = find_table(&tables, &hdr.tables, HEAD_TABLE_TAG) {
        let head_table = hdr.tables[head];
        if head_table.dst_length < 12 {
            return false;
        }
        let checksum_adjustment = 0xB1B0_AFBAu32.wrapping_sub(font_checksum).to_be_bytes();
        if !out.write_at(
            &checksum_adjustment,
            head_table.dst_offset.wrapping_add(8) as usize,
        ) {
            return false;
        }
    }

    true
}

fn read_woff2_header(data: &[u8], hdr: &mut Woff2Header) -> bool {
    let length = data.len();
    let mut file = Buffer::new(data);

    match file.read_u32() {
        Some(WOFF2_SIGNATURE) => {}
        _ => return false,
    }
    match file.read_u32() {
        Some(v) => hdr.flavor = v,
        None => return false,
    }

    // TODO(user): Should call IsValidVersionTag() here.

    match file.read_u32() {
        Some(reported_length) if reported_length as usize == length => {}
        _ => return false,
    }
    match file.read_u16() {
        Some(n) if n != 0 => hdr.num_tables = n,
        Some(n) => {
            hdr.num_tables = n;
            return false;
        }
        None => return false,
    }

    // We don't care about these fields of the header:
    //   uint16_t reserved
    //   uint32_t total_sfnt_size, we don't believe this, will compute later
    if !file.skip(6) {
        return false;
    }
    match file.read_u32() {
        Some(v) => hdr.compressed_length = v,
        None => return false,
    }
    // We don't care about these fields of the header:
    //   uint16_t major_version, minor_version
    if !file.skip(2 * 2) {
        return false;
    }
    let (meta_offset, meta_length) = match (file.read_u32(), file.read_u32(), file.read_u32()) {
        (Some(o), Some(l), Some(_orig)) => (o, l),
        _ => return false,
    };
    if meta_offset != 0
        && (meta_offset as usize >= length
            || length - (meta_offset as usize) < meta_length as usize)
    {
        return false;
    }
    let (priv_offset, priv_length) = match (file.read_u32(), file.read_u32()) {
        (Some(o), Some(l)) => (o, l),
        _ => return false,
    };
    if priv_offset != 0
        && (priv_offset as usize >= length
            || length - (priv_offset as usize) < priv_length as usize)
    {
        return false;
    }
    hdr.tables = vec![Table::default(); usize::from(hdr.num_tables)];
    if !read_table_directory(&mut file, &mut hdr.tables) {
        return false;
    }

    // Before we sort for output the last table end is the uncompressed size.
    let last_table = *hdr.tables.last().expect("num_tables > 0");
    hdr.uncompressed_size = last_table.src_offset.wrapping_add(last_table.src_length);
    if hdr.uncompressed_size < last_table.src_offset {
        return false;
    }

    hdr.header_version = 0;

    if hdr.flavor == TTC_FONT_FLAVOR {
        match file.read_u32() {
            Some(v) => hdr.header_version = v,
            None => return false,
        }
        if hdr.header_version != 0x0001_0000 && hdr.header_version != 0x0002_0000 {
            return false;
        }
        let num_fonts = match read_255_ushort(&mut file) {
            Some(n) if n != 0 => n,
            _ => return false,
        };
        hdr.ttc_fonts = vec![TtcFont::default(); num_fonts as usize];

        for i in 0..num_fonts as usize {
            let num_tables = match read_255_ushort(&mut file) {
                Some(n) if n != 0 => n,
                _ => return false,
            };
            match file.read_u32() {
                Some(v) => hdr.ttc_fonts[i].flavor = v,
                None => return false,
            }

            hdr.ttc_fonts[i].table_indices = vec![0u16; num_tables as usize];

            let mut glyf_idx: u32 = 0;
            let mut loca_idx: u32 = 0;

            for j in 0..num_tables as usize {
                let table_idx = match read_255_ushort(&mut file) {
                    Some(v) if (v as usize) < hdr.tables.len() => v,
                    _ => return false,
                };
                hdr.ttc_fonts[i].table_indices[j] = table_idx as u16;

                let table = &hdr.tables[table_idx as usize];
                if table.tag == LOCA_TABLE_TAG {
                    loca_idx = table_idx;
                }
                if table.tag == GLYF_TABLE_TAG {
                    glyf_idx = table_idx;
                }
            }

            // if we have both glyf and loca make sure they are consecutive
            // if we have just one we'll reject the font elsewhere
            if (glyf_idx > 0 || loca_idx > 0) && (glyf_idx > loca_idx || loca_idx - glyf_idx != 1) {
                noisy!("TTC font {} has non-consecutive glyf/loca\n", i as i32);
                return false;
            }
        }
    }

    let first_table_offset = compute_offset_to_first_table(hdr);

    hdr.compressed_offset = file.offset() as u64;
    if hdr.compressed_offset > u64::from(u32::MAX) {
        return false;
    }
    let mut src_offset = round4_u64(hdr.compressed_offset + u64::from(hdr.compressed_length));

    if src_offset > length as u64 {
        noisy!(
            "offset fail; src_offset {} length {} dst_offset {}\n",
            src_offset,
            length,
            first_table_offset
        );
        return false;
    }
    if meta_offset != 0 {
        if src_offset != u64::from(meta_offset) {
            return false;
        }
        src_offset = u64::from(round4_u32(meta_offset.wrapping_add(meta_length)));
        if src_offset > u64::from(u32::MAX) {
            return false;
        }
    }

    if priv_offset != 0 {
        if src_offset != u64::from(priv_offset) {
            return false;
        }
        src_offset = u64::from(round4_u32(priv_offset.wrapping_add(priv_length)));
        if src_offset > u64::from(u32::MAX) {
            return false;
        }
    }

    if src_offset != round4_usize(length) as u64 {
        return false;
    }

    true
}

/// Writes everything before the actual table data.
fn write_headers(
    metadata: &mut RebuildMetadata,
    hdr: &mut Woff2Header,
    out: &mut dyn Woff2Out,
) -> bool {
    let first_table_offset = compute_offset_to_first_table(hdr);
    let mut output: Vec<u8> = Vec::new();
    if output
        .try_reserve_exact(first_table_offset as usize)
        .is_err()
    {
        return false;
    }
    output.resize(first_table_offset as usize, 0);

    // Re-order tables in output (OTSpec) order
    let mut sorted_tables: Vec<Table> = hdr.tables.clone();
    if hdr.header_version != 0 {
        // collection; we have to sort the table offset vector in each font
        for ttc_font in &mut hdr.ttc_fonts {
            let mut sorted_index_by_tag: BTreeMap<u32, u16> = BTreeMap::new();
            for &table_index in &ttc_font.table_indices {
                sorted_index_by_tag.insert(hdr.tables[usize::from(table_index)].tag, table_index);
            }
            for (index, (_, &i)) in sorted_index_by_tag.iter().enumerate() {
                ttc_font.table_indices[index] = i;
            }
        }
    } else {
        // non-collection; we can just sort the tables
        sorted_tables.sort_by_key(|t| t.tag);
    }

    // Start building the font
    let result = &mut output[..];
    let mut offset: usize = 0;
    if hdr.header_version != 0 {
        // TTC header
        offset = store_u32(result, offset, hdr.flavor); // TAG TTCTag
        offset = store_u32(result, offset, hdr.header_version); // FIXED Version
        offset = store_u32(result, offset, hdr.ttc_fonts.len() as u32); // ULONG numFonts
                                                                        // Space for ULONG OffsetTable[numFonts] (zeroed initially)
        let mut offset_table = offset; // keep start of offset table for later
        for _ in 0..hdr.ttc_fonts.len() {
            offset = store_u32(result, offset, 0); // will fill real values in later
        }
        // space for DSIG fields for header v2
        if hdr.header_version == 0x0002_0000 {
            offset = store_u32(result, offset, 0); // ULONG ulDsigTag
            offset = store_u32(result, offset, 0); // ULONG ulDsigLength
            offset = store_u32(result, offset, 0); // ULONG ulDsigOffset
        }

        // write Offset Tables and store the location of each in TTC Header
        metadata
            .font_infos
            .resize_with(hdr.ttc_fonts.len(), Woff2FontInfo::default);
        for i in 0..hdr.ttc_fonts.len() {
            let ttc_font = &mut hdr.ttc_fonts[i];

            // write Offset Table location into TTC Header
            offset_table = store_u32(result, offset_table, offset as u32);

            // write the actual offset table so our header doesn't lie
            ttc_font.dst_offset = offset as u32;
            offset = store_offset_table(
                result,
                offset,
                ttc_font.flavor,
                ttc_font.table_indices.len() as u16,
            );

            for &table_index in &ttc_font.table_indices {
                let tag = hdr.tables[usize::from(table_index)].tag;
                metadata.font_infos[i]
                    .table_entry_by_tag
                    .insert(tag, offset as u32);
                offset = store_table_entry(result, offset, tag);
            }

            let start = ttc_font.dst_offset as usize;
            ttc_font.header_checksum = compute_ulong_sum(&result[start..offset]);
        }
    } else {
        metadata.font_infos.resize_with(1, Woff2FontInfo::default);
        offset = store_offset_table(result, offset, hdr.flavor, hdr.num_tables);
        for table in sorted_tables.iter().take(usize::from(hdr.num_tables)) {
            metadata.font_infos[0]
                .table_entry_by_tag
                .insert(table.tag, offset as u32);
            offset = store_table_entry(result, offset, table.tag);
        }
    }

    if !out.write(&output) {
        return false;
    }
    metadata.header_checksum = compute_ulong_sum(&output);
    true
}

/// Computes the size of the final uncompressed font, or 0 on error.
///
/// This simply returns the `totalSfntSize` field of the WOFF2 header.
pub fn compute_woff2_final_size(data: &[u8]) -> usize {
    let mut file = Buffer::new(data);
    if !file.skip(16) {
        return 0;
    }
    match file.read_u32() {
        Some(total_length) => total_length as usize,
        None => 0,
    }
}

/// Decompresses the font into `out`, using `decompress` (which must behave
/// like `BrotliDecoderDecompress`). Returns true on success.
///
/// Works even if the WOFF2 header's `totalSfntSize` is wrong.
pub fn convert_woff2_to_ttf_with(
    data: &[u8],
    out: &mut dyn Woff2Out,
    decompress: DecompressFn,
) -> bool {
    let mut metadata = RebuildMetadata::default();
    let mut hdr = Woff2Header::default();
    if !read_woff2_header(data, &mut hdr) {
        return false;
    }

    if !write_headers(&mut metadata, &mut hdr, out) {
        return false;
    }

    let compression_ratio = hdr.uncompressed_size as f32 / data.len() as f32;
    if compression_ratio > MAX_PLAUSIBLE_COMPRESSION_RATIO {
        noisy!(
            "Implausible compression ratio {:.1}\n",
            f64::from(compression_ratio)
        );
        return false;
    }

    let start = hdr.compressed_offset as usize;
    let src_buf = &data[start..start + hdr.compressed_length as usize];
    let mut uncompressed_buf: Vec<u8> = Vec::new();
    if uncompressed_buf
        .try_reserve_exact(hdr.uncompressed_size as usize)
        .is_err()
    {
        return false;
    }
    uncompressed_buf.resize(hdr.uncompressed_size as usize, 0);
    if hdr.uncompressed_size < 1 {
        return false;
    }
    if !brotli::decompress_exact(decompress, src_buf, &mut uncompressed_buf) {
        return false;
    }

    for i in 0..metadata.font_infos.len() {
        if !reconstruct_font(&mut uncompressed_buf, &mut metadata, &mut hdr, i, out) {
            return false;
        }
    }

    true
}

/// Decompresses the font into `out` using the system brotli library.
#[cfg(feature = "system-brotli")]
pub fn convert_woff2_to_ttf(data: &[u8], out: &mut dyn Woff2Out) -> bool {
    convert_woff2_to_ttf_with(data, out, brotli::SYSTEM_DECOMPRESS)
}

/// Decompresses the font into `result` (deprecated C++ API shape). The
/// buffer should be sized according to [`compute_woff2_final_size`].
#[cfg(feature = "system-brotli")]
pub fn convert_woff2_to_ttf_buf(result: &mut [u8], data: &[u8]) -> bool {
    let mut out = crate::output::MemoryOut::new(result);
    convert_woff2_to_ttf(data, &mut out)
}
