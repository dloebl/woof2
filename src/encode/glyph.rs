//! Glyph parsing and serialization (`glyph.cc`).

// Index loops are kept to mirror the C++ code line by line.
#![allow(clippy::needless_range_loop)]

use crate::buffer::Buffer;

const FLAG_ONCURVE: i32 = 1;
const FLAG_XSHORT: i32 = 1 << 1;
const FLAG_YSHORT: i32 = 1 << 2;
const FLAG_REPEAT: i32 = 1 << 3;
const FLAG_XREPEATSIGN: i32 = 1 << 4;
const FLAG_YREPEATSIGN: i32 = 1 << 5;
const FLAG_OVERLAP_SIMPLE: i32 = 1 << 6;
const FLAG_ARG_1_AND_2_ARE_WORDS: u16 = 1 << 0;
const FLAG_WE_HAVE_A_SCALE: u16 = 1 << 3;
const FLAG_MORE_COMPONENTS: u16 = 1 << 5;
const FLAG_WE_HAVE_AN_X_AND_Y_SCALE: u16 = 1 << 6;
const FLAG_WE_HAVE_A_TWO_BY_TWO: u16 = 1 << 7;
const FLAG_WE_HAVE_INSTRUCTIONS: u16 = 1 << 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Point {
    pub x: i32,
    pub y: i32,
    pub on_curve: bool,
}

/// A parsed simple or composite glyph. Composite glyph data and instructions
/// are not parsed; they borrow the raw glyph data.
#[derive(Clone, Debug, Default)]
pub struct Glyph<'a> {
    // Bounding box.
    pub x_min: i16,
    pub x_max: i16,
    pub y_min: i16,
    pub y_max: i16,

    // Instructions.
    pub instructions_size: u16,
    pub instructions_data: &'a [u8],

    // Flags.
    pub overlap_simple_flag_set: bool,

    // Data model for simple glyphs.
    pub contours: Vec<Vec<Point>>,

    // Data for composite glyphs.
    pub composite_data: &'a [u8],
    pub composite_data_size: u32,
    pub have_instructions: bool,
}

fn read_composite_glyph_data<'a>(buffer: &mut Buffer<'a>, glyph: &mut Glyph<'a>) -> bool {
    glyph.have_instructions = false;
    let start_offset = buffer.offset();
    let mut flags = FLAG_MORE_COMPONENTS;
    while flags & FLAG_MORE_COMPONENTS != 0 {
        flags = match buffer.read_u16() {
            Some(v) => v,
            None => return false,
        };
        glyph.have_instructions |= (flags & FLAG_WE_HAVE_INSTRUCTIONS) != 0;
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
        if !buffer.skip(arg_size) {
            return false;
        }
    }
    let size = buffer.offset() - start_offset;
    if size > u32::MAX as usize {
        return false;
    }
    glyph.composite_data = &buffer.buffer()[start_offset..buffer.offset()];
    glyph.composite_data_size = size as u32;
    true
}

/// Parses the glyph from `data`.
pub fn read_glyph<'a>(data: &'a [u8], glyph: &mut Glyph<'a>) -> bool {
    let mut buffer = Buffer::new(data);

    let num_contours = match buffer.read_s16() {
        Some(v) => v,
        None => return false,
    };

    // Read the bounding box.
    match buffer.read_s16() {
        Some(v) => glyph.x_min = v,
        None => return false,
    }
    match buffer.read_s16() {
        Some(v) => glyph.y_min = v,
        None => return false,
    }
    match buffer.read_s16() {
        Some(v) => glyph.x_max = v,
        None => return false,
    }
    match buffer.read_s16() {
        Some(v) => glyph.y_max = v,
        None => return false,
    }

    if num_contours == 0 {
        // Empty glyph.
        return true;
    }

    if num_contours > 0 {
        // Simple glyph.
        let num_contours = num_contours as usize;
        glyph.contours.resize(num_contours, Vec::new());

        // Read the number of points per contour.
        let mut last_point_index: u16 = 0;
        for i in 0..num_contours {
            let point_index = match buffer.read_u16() {
                Some(v) => v,
                None => return false,
            };
            if point_index < last_point_index || (point_index == 0xFFFF && i == 0) {
                return false;
            }
            let num_points = point_index
                .wrapping_sub(last_point_index)
                .wrapping_add(if i == 0 { 1 } else { 0 });
            glyph.contours[i].resize(usize::from(num_points), Point::default());
            last_point_index = point_index;
        }

        // Read the instructions.
        match buffer.read_u16() {
            Some(v) => glyph.instructions_size = v,
            None => return false,
        }
        match buffer.read(usize::from(glyph.instructions_size)) {
            Some(s) => glyph.instructions_data = s,
            None => return false,
        }

        // Read the run-length coded flags.
        let mut flags: Vec<Vec<u8>> = vec![Vec::new(); num_contours];
        {
            let mut flag: u8 = 0;
            let mut flag_repeat: u8 = 0;
            for i in 0..num_contours {
                flags[i].resize(glyph.contours[i].len(), 0);
                for j in 0..glyph.contours[i].len() {
                    if flag_repeat == 0 {
                        match buffer.read_u8() {
                            Some(v) => flag = v,
                            None => return false,
                        }
                        if i32::from(flag) & FLAG_REPEAT != 0 {
                            match buffer.read_u8() {
                                Some(v) => flag_repeat = v,
                                None => return false,
                            }
                        }
                    } else {
                        flag_repeat -= 1;
                    }
                    flags[i][j] = flag;
                    glyph.contours[i][j].on_curve = i32::from(flag) & FLAG_ONCURVE != 0;
                }
            }
        }

        if let Some(&first) = flags.first().and_then(|f| f.first()) {
            glyph.overlap_simple_flag_set = i32::from(first) & FLAG_OVERLAP_SIMPLE != 0;
        }

        // Read the x coordinates.
        let mut prev_x: i32 = 0;
        for i in 0..num_contours {
            for j in 0..glyph.contours[i].len() {
                let flag = i32::from(flags[i][j]);
                if flag & FLAG_XSHORT != 0 {
                    // single byte x-delta coord value
                    let x_delta = match buffer.read_u8() {
                        Some(v) => i32::from(v),
                        None => return false,
                    };
                    let sign = if flag & FLAG_XREPEATSIGN != 0 { 1 } else { -1 };
                    glyph.contours[i][j].x = prev_x.wrapping_add(sign * x_delta);
                } else {
                    // double byte x-delta coord value
                    let mut x_delta: i16 = 0;
                    if flag & FLAG_XREPEATSIGN == 0 {
                        match buffer.read_s16() {
                            Some(v) => x_delta = v,
                            None => return false,
                        }
                    }
                    glyph.contours[i][j].x = prev_x.wrapping_add(i32::from(x_delta));
                }
                prev_x = glyph.contours[i][j].x;
            }
        }

        // Read the y coordinates.
        let mut prev_y: i32 = 0;
        for i in 0..num_contours {
            for j in 0..glyph.contours[i].len() {
                let flag = i32::from(flags[i][j]);
                if flag & FLAG_YSHORT != 0 {
                    // single byte y-delta coord value
                    let y_delta = match buffer.read_u8() {
                        Some(v) => i32::from(v),
                        None => return false,
                    };
                    let sign = if flag & FLAG_YREPEATSIGN != 0 { 1 } else { -1 };
                    glyph.contours[i][j].y = prev_y.wrapping_add(sign * y_delta);
                } else {
                    // double byte y-delta coord value
                    let mut y_delta: i16 = 0;
                    if flag & FLAG_YREPEATSIGN == 0 {
                        match buffer.read_s16() {
                            Some(v) => y_delta = v,
                            None => return false,
                        }
                    }
                    glyph.contours[i][j].y = prev_y.wrapping_add(i32::from(y_delta));
                }
                prev_y = glyph.contours[i][j].y;
            }
        }
    } else if num_contours == -1 {
        // Composite glyph.
        if !read_composite_glyph_data(&mut buffer, glyph) {
            return false;
        }
        // Read the instructions.
        if glyph.have_instructions {
            match buffer.read_u16() {
                Some(v) => glyph.instructions_size = v,
                None => return false,
            }
            match buffer.read(usize::from(glyph.instructions_size)) {
                Some(s) => glyph.instructions_data = s,
                None => return false,
            }
        } else {
            glyph.instructions_size = 0;
        }
    } else {
        return false;
    }
    true
}

#[inline]
fn put16(dst: &mut [u8], offset: &mut usize, v: i32) {
    dst[*offset] = (v >> 8) as u8;
    dst[*offset + 1] = v as u8;
    *offset += 2;
}

fn store_bbox(glyph: &Glyph<'_>, offset: &mut usize, dst: &mut [u8]) {
    put16(dst, offset, i32::from(glyph.x_min));
    put16(dst, offset, i32::from(glyph.y_min));
    put16(dst, offset, i32::from(glyph.x_max));
    put16(dst, offset, i32::from(glyph.y_max));
}

fn store_instructions(glyph: &Glyph<'_>, offset: &mut usize, dst: &mut [u8]) {
    put16(dst, offset, i32::from(glyph.instructions_size));
    let n = usize::from(glyph.instructions_size);
    dst[*offset..*offset + n].copy_from_slice(&glyph.instructions_data[..n]);
    *offset += n;
}

fn store_end_pts_of_contours(glyph: &Glyph<'_>, offset: &mut usize, dst: &mut [u8]) -> bool {
    let mut end_point: i32 = -1;
    for contour in &glyph.contours {
        end_point = (end_point as usize).wrapping_add(contour.len()) as i32;
        if contour.len() > usize::from(u16::MAX) || end_point > i32::from(u16::MAX) {
            return false;
        }
        put16(dst, offset, end_point);
    }
    true
}

fn store_points(glyph: &Glyph<'_>, offset: &mut usize, dst: &mut [u8]) -> bool {
    let dst_size = dst.len();
    let mut previous_flag: i32 = -1;
    let mut repeat_count: i32 = 0;
    let mut last_x: i32 = 0;
    let mut last_y: i32 = 0;
    let mut x_bytes: usize = 0;
    let mut y_bytes: usize = 0;

    // Store the flags and calculate the total size of the x and y coordinates.
    for contour in &glyph.contours {
        for point in contour {
            let mut flag = if point.on_curve { FLAG_ONCURVE } else { 0 };
            if previous_flag == -1 && glyph.overlap_simple_flag_set {
                // First flag needs to have overlap simple bit set.
                flag |= FLAG_OVERLAP_SIMPLE;
            }
            let dx = point.x.wrapping_sub(last_x);
            let dy = point.y.wrapping_sub(last_y);
            if dx == 0 {
                flag |= FLAG_XREPEATSIGN;
            } else if dx > -256 && dx < 256 {
                flag |= FLAG_XSHORT | if dx > 0 { FLAG_XREPEATSIGN } else { 0 };
                x_bytes += 1;
            } else {
                x_bytes += 2;
            }
            if dy == 0 {
                flag |= FLAG_YREPEATSIGN;
            } else if dy > -256 && dy < 256 {
                flag |= FLAG_YSHORT | if dy > 0 { FLAG_YREPEATSIGN } else { 0 };
                y_bytes += 1;
            } else {
                y_bytes += 2;
            }
            if flag == previous_flag && repeat_count != 255 {
                dst[*offset - 1] |= FLAG_REPEAT as u8;
                repeat_count += 1;
            } else {
                if repeat_count != 0 {
                    if *offset >= dst_size {
                        return false;
                    }
                    dst[*offset] = repeat_count as u8;
                    *offset += 1;
                }
                if *offset >= dst_size {
                    return false;
                }
                dst[*offset] = flag as u8;
                *offset += 1;
                repeat_count = 0;
            }
            last_x = point.x;
            last_y = point.y;
            previous_flag = flag;
        }
    }
    if repeat_count != 0 {
        if *offset >= dst_size {
            return false;
        }
        dst[*offset] = repeat_count as u8;
        *offset += 1;
    }

    if *offset + x_bytes + y_bytes > dst_size {
        return false;
    }

    // Store the x and y coordinates.
    let mut x_offset = *offset;
    let mut y_offset = *offset + x_bytes;
    last_x = 0;
    last_y = 0;
    for contour in &glyph.contours {
        for point in contour {
            let dx = point.x.wrapping_sub(last_x);
            let dy = point.y.wrapping_sub(last_y);
            if dx == 0 {
                // pass
            } else if dx > -256 && dx < 256 {
                dst[x_offset] = dx.unsigned_abs() as u8;
                x_offset += 1;
            } else {
                put16(dst, &mut x_offset, dx);
            }
            if dy == 0 {
                // pass
            } else if dy > -256 && dy < 256 {
                dst[y_offset] = dy.unsigned_abs() as u8;
                y_offset += 1;
            } else {
                put16(dst, &mut y_offset, dy);
            }
            last_x = last_x.wrapping_add(dx);
            last_y = last_y.wrapping_add(dy);
        }
    }
    *offset = y_offset;
    true
}

/// Stores the glyph into `dst`. Returns the actual (unpadded) stored size,
/// or `None` if `dst` is too small.
pub fn store_glyph(glyph: &Glyph<'_>, dst: &mut [u8]) -> Option<usize> {
    let dst_size = dst.len() as u64;
    let mut offset: usize = 0;
    if glyph.composite_data_size > 0 {
        // Composite glyph.
        let needed = (10u64 + u64::from(glyph.composite_data_size))
            + (if glyph.have_instructions { 2u64 } else { 0 } + u64::from(glyph.instructions_size));
        if dst_size < needed {
            return None;
        }
        put16(dst, &mut offset, -1);
        store_bbox(glyph, &mut offset, dst);
        let n = glyph.composite_data_size as usize;
        dst[offset..offset + n].copy_from_slice(&glyph.composite_data[..n]);
        offset += n;
        if glyph.have_instructions {
            store_instructions(glyph, &mut offset, dst);
        }
    } else if !glyph.contours.is_empty() {
        // Simple glyph.
        if glyph.contours.len() > i16::MAX as usize {
            return None;
        }
        if dst_size < (12u64 + 2 * glyph.contours.len() as u64) + u64::from(glyph.instructions_size)
        {
            return None;
        }
        put16(dst, &mut offset, glyph.contours.len() as i32);
        store_bbox(glyph, &mut offset, dst);
        if !store_end_pts_of_contours(glyph, &mut offset, dst) {
            return None;
        }
        store_instructions(glyph, &mut offset, dst);
        if !store_points(glyph, &mut offset, dst) {
            return None;
        }
    }
    Some(offset)
}
