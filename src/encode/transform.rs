//! Font preprocessing as part of the WOFF 2.0 conversion (`transform.cc`).
//!
//! Only the glyf/loca transform is implemented: the reference encoder never
//! applies the optional hmtx transform (`TransformHmtxTable` is dead code).

use crate::table_tags::{GLYF_TABLE_TAG, HEAD_TABLE_TAG, LOCA_TABLE_TAG};
use crate::variable_length::write_255_ushort;

use super::font::{get_glyph_data, num_glyphs, DataRef, Font};
use super::glyph::{read_glyph, Glyph};

const FLAG_OVERLAP_SIMPLE_BITMAP: i32 = 1 << 0;

#[inline]
fn write_ushort(out: &mut Vec<u8>, value: i32) {
    out.push((value >> 8) as u8);
    out.push((value & 255) as u8);
}

#[inline]
fn write_long(out: &mut Vec<u8>, value: i32) {
    out.push(((value >> 24) & 255) as u8);
    out.push(((value >> 16) & 255) as u8);
    out.push(((value >> 8) & 255) as u8);
    out.push((value & 255) as u8);
}

/// Glyf table preprocessing, based on GlyfEncoder.java.
struct GlyfEncoder {
    n_contour_stream: Vec<u8>,
    n_points_stream: Vec<u8>,
    flag_byte_stream: Vec<u8>,
    composite_stream: Vec<u8>,
    bbox_bitmap: Vec<u8>,
    bbox_stream: Vec<u8>,
    glyph_stream: Vec<u8>,
    instruction_stream: Vec<u8>,
    overlap_bitmap: Vec<u8>,
    n_glyphs: i32,
}

impl GlyfEncoder {
    fn new(num_glyphs: i32) -> Self {
        GlyfEncoder {
            n_contour_stream: Vec::new(),
            n_points_stream: Vec::new(),
            flag_byte_stream: Vec::new(),
            composite_stream: Vec::new(),
            bbox_bitmap: vec![0u8; (((num_glyphs.wrapping_add(31)) >> 5) << 2) as usize],
            bbox_stream: Vec::new(),
            glyph_stream: Vec::new(),
            instruction_stream: Vec::new(),
            overlap_bitmap: Vec::new(),
            n_glyphs: num_glyphs,
        }
    }

    fn encode(&mut self, glyph_id: i32, glyph: &Glyph<'_>) {
        if glyph.composite_data_size > 0 {
            self.write_composite_glyph(glyph_id, glyph);
        } else if !glyph.contours.is_empty() {
            self.write_simple_glyph(glyph_id, glyph);
        } else {
            write_ushort(&mut self.n_contour_stream, 0);
        }
    }

    fn get_transformed_glyf_bytes(&self, result: &mut Vec<u8>) {
        // The size is known exactly; reserving it avoids the up to 2x
        // capacity slack of growing the buffer stream by stream (the buffer
        // stays alive until the end of the conversion).
        let streams = [
            &self.n_contour_stream,
            &self.n_points_stream,
            &self.flag_byte_stream,
            &self.glyph_stream,
            &self.composite_stream,
            &self.bbox_bitmap,
            &self.bbox_stream,
            &self.instruction_stream,
            &self.overlap_bitmap,
        ];
        result.reserve_exact(36 + streams.iter().map(|s| s.len()).sum::<usize>());
        write_ushort(result, 0); // Version
        write_ushort(
            result,
            if self.overlap_bitmap.is_empty() {
                0x00
            } else {
                FLAG_OVERLAP_SIMPLE_BITMAP
            },
        ); // Flags
        write_ushort(result, self.n_glyphs);
        write_ushort(result, 0); // index_format, will be set later
        write_long(result, self.n_contour_stream.len() as i32);
        write_long(result, self.n_points_stream.len() as i32);
        write_long(result, self.flag_byte_stream.len() as i32);
        write_long(result, self.glyph_stream.len() as i32);
        write_long(result, self.composite_stream.len() as i32);
        write_long(
            result,
            (self.bbox_bitmap.len() + self.bbox_stream.len()) as i32,
        );
        write_long(result, self.instruction_stream.len() as i32);
        result.extend_from_slice(&self.n_contour_stream);
        result.extend_from_slice(&self.n_points_stream);
        result.extend_from_slice(&self.flag_byte_stream);
        result.extend_from_slice(&self.glyph_stream);
        result.extend_from_slice(&self.composite_stream);
        result.extend_from_slice(&self.bbox_bitmap);
        result.extend_from_slice(&self.bbox_stream);
        result.extend_from_slice(&self.instruction_stream);
        if !self.overlap_bitmap.is_empty() {
            result.extend_from_slice(&self.overlap_bitmap);
        }
    }

    fn write_instructions(&mut self, glyph: &Glyph<'_>) {
        write_255_ushort(&mut self.glyph_stream, i32::from(glyph.instructions_size));
        let n = usize::from(glyph.instructions_size);
        self.instruction_stream
            .extend_from_slice(&glyph.instructions_data[..n]);
    }

    fn should_write_simple_glyph_bbox(glyph: &Glyph<'_>) -> bool {
        if glyph.contours.is_empty() || glyph.contours[0].is_empty() {
            return glyph.x_min != 0 || glyph.y_min != 0 || glyph.x_max != 0 || glyph.y_max != 0;
        }

        let mut x_min = glyph.contours[0][0].x as i16;
        let mut y_min = glyph.contours[0][0].y as i16;
        let mut x_max = x_min;
        let mut y_max = y_min;
        for contour in &glyph.contours {
            for point in contour {
                if point.x < i32::from(x_min) {
                    x_min = point.x as i16;
                }
                if point.x > i32::from(x_max) {
                    x_max = point.x as i16;
                }
                if point.y < i32::from(y_min) {
                    y_min = point.y as i16;
                }
                if point.y > i32::from(y_max) {
                    y_max = point.y as i16;
                }
            }
        }

        glyph.x_min != x_min || glyph.y_min != y_min || glyph.x_max != x_max || glyph.y_max != y_max
    }

    fn write_simple_glyph(&mut self, glyph_id: i32, glyph: &Glyph<'_>) {
        if glyph.overlap_simple_flag_set {
            self.ensure_overlap_bitmap();
            self.overlap_bitmap[(glyph_id >> 3) as usize] |= 0x80 >> (glyph_id & 7);
        }
        let num_contours = glyph.contours.len() as i32;
        write_ushort(&mut self.n_contour_stream, num_contours);
        if Self::should_write_simple_glyph_bbox(glyph) {
            self.write_bbox(glyph_id, glyph);
        }
        for contour in &glyph.contours {
            write_255_ushort(&mut self.n_points_stream, contour.len() as i32);
        }
        let mut last_x: i32 = 0;
        let mut last_y: i32 = 0;
        for contour in &glyph.contours {
            for point in contour {
                let dx = point.x.wrapping_sub(last_x);
                let dy = point.y.wrapping_sub(last_y);
                self.write_triplet(point.on_curve, dx, dy);
                last_x = point.x;
                last_y = point.y;
            }
        }
        if num_contours > 0 {
            self.write_instructions(glyph);
        }
    }

    fn write_composite_glyph(&mut self, glyph_id: i32, glyph: &Glyph<'_>) {
        write_ushort(&mut self.n_contour_stream, -1);
        self.write_bbox(glyph_id, glyph);
        let n = glyph.composite_data_size as usize;
        self.composite_stream
            .extend_from_slice(&glyph.composite_data[..n]);
        if glyph.have_instructions {
            self.write_instructions(glyph);
        }
    }

    fn write_bbox(&mut self, glyph_id: i32, glyph: &Glyph<'_>) {
        self.bbox_bitmap[(glyph_id >> 3) as usize] |= 0x80 >> (glyph_id & 7);
        write_ushort(&mut self.bbox_stream, i32::from(glyph.x_min));
        write_ushort(&mut self.bbox_stream, i32::from(glyph.y_min));
        write_ushort(&mut self.bbox_stream, i32::from(glyph.x_max));
        write_ushort(&mut self.bbox_stream, i32::from(glyph.y_max));
    }

    fn write_triplet(&mut self, on_curve: bool, x: i32, y: i32) {
        let abs_x = x.wrapping_abs();
        let abs_y = y.wrapping_abs();
        let on_curve_bit = if on_curve { 0 } else { 128 };
        let x_sign_bit = if x < 0 { 0 } else { 1 };
        let y_sign_bit = if y < 0 { 0 } else { 1 };
        let xy_sign_bits = x_sign_bit + 2 * y_sign_bit;
        let flags = &mut self.flag_byte_stream;
        let glyphs = &mut self.glyph_stream;
        if x == 0 && abs_y < 1280 {
            flags.push((on_curve_bit + ((abs_y & 0xf00) >> 7) + y_sign_bit) as u8);
            glyphs.push((abs_y & 0xff) as u8);
        } else if y == 0 && abs_x < 1280 {
            flags.push((on_curve_bit + 10 + ((abs_x & 0xf00) >> 7) + x_sign_bit) as u8);
            glyphs.push((abs_x & 0xff) as u8);
        } else if abs_x < 65 && abs_y < 65 {
            flags.push(
                (on_curve_bit
                    + 20
                    + ((abs_x - 1) & 0x30)
                    + (((abs_y - 1) & 0x30) >> 2)
                    + xy_sign_bits) as u8,
            );
            glyphs.push(((((abs_x - 1) & 0xf) << 4) | ((abs_y - 1) & 0xf)) as u8);
        } else if abs_x < 769 && abs_y < 769 {
            flags.push(
                (on_curve_bit
                    + 84
                    + 12 * (((abs_x - 1) & 0x300) >> 8)
                    + (((abs_y - 1) & 0x300) >> 6)
                    + xy_sign_bits) as u8,
            );
            glyphs.push(((abs_x - 1) & 0xff) as u8);
            glyphs.push(((abs_y - 1) & 0xff) as u8);
        } else if abs_x < 4096 && abs_y < 4096 {
            flags.push((on_curve_bit + 120 + xy_sign_bits) as u8);
            glyphs.push((abs_x >> 4) as u8);
            glyphs.push((((abs_x & 0xf) << 4) | (abs_y >> 8)) as u8);
            glyphs.push((abs_y & 0xff) as u8);
        } else {
            flags.push((on_curve_bit + 124 + xy_sign_bits) as u8);
            glyphs.push((abs_x >> 8) as u8);
            glyphs.push((abs_x & 0xff) as u8);
            glyphs.push((abs_y >> 8) as u8);
            glyphs.push((abs_y & 0xff) as u8);
        }
    }

    fn ensure_overlap_bitmap(&mut self) {
        if self.overlap_bitmap.is_empty() {
            self.overlap_bitmap = vec![0u8; ((self.n_glyphs.wrapping_add(7)) >> 3) as usize];
        }
    }
}

/// Adds the transformed versions of the glyf and loca tables to the font.
/// The transformed loca table has zero length. The tag of the transformed
/// tables is derived from the original tag by flipping the MSBs of every
/// byte.
pub fn transform_glyf_and_loca_tables(font: &mut Font, input: &[u8]) -> bool {
    // no transform for CFF
    let (glyf, loca) = (
        font.find_table(GLYF_TABLE_TAG),
        font.find_table(LOCA_TABLE_TAG),
    );

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

    font.tables.entry(GLYF_TABLE_TAG ^ 0x8080_8080).or_default();
    font.tables.entry(LOCA_TABLE_TAG ^ 0x8080_8080).or_default();

    let num_glyphs = num_glyphs(font, input);
    let mut encoder = GlyfEncoder::new(num_glyphs);
    for i in 0..num_glyphs {
        let glyph_data = match get_glyph_data(font, i, input) {
            Some(d) => d,
            None => return false,
        };
        let mut glyph = Glyph::default();
        if !glyph_data.is_empty() && !read_glyph(glyph_data, &mut glyph) {
            return false;
        }
        encoder.encode(i, &glyph);
    }
    let mut transformed = std::mem::take(
        &mut font
            .tables
            .get_mut(&(GLYF_TABLE_TAG ^ 0x8080_8080))
            .unwrap()
            .buffer,
    );
    encoder.get_transformed_glyf_bytes(&mut transformed);

    let index_format = match font.find_table(HEAD_TABLE_TAG) {
        Some(head) if head.length >= 52 => head.data(input).get(51).copied().unwrap_or(0),
        _ => {
            font.tables
                .get_mut(&(GLYF_TABLE_TAG ^ 0x8080_8080))
                .unwrap()
                .buffer = transformed;
            return false;
        }
    };
    transformed[7] = index_format;

    let transformed_glyf = font
        .tables
        .get_mut(&(GLYF_TABLE_TAG ^ 0x8080_8080))
        .unwrap();
    transformed_glyf.tag = GLYF_TABLE_TAG ^ 0x8080_8080;
    transformed_glyf.length = transformed.len() as u32;
    transformed_glyf.buffer = transformed;
    transformed_glyf.data = DataRef::Buffer;

    let transformed_loca = font
        .tables
        .get_mut(&(LOCA_TABLE_TAG ^ 0x8080_8080))
        .unwrap();
    transformed_loca.tag = LOCA_TABLE_TAG ^ 0x8080_8080;
    transformed_loca.length = 0;
    transformed_loca.data = DataRef::Null;

    true
}
