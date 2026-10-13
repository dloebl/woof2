//! A commandline tool for dumping info about a woff2 file.

#[path = "util.rs"]
mod util;

use std::io::Write;
use std::process::ExitCode;

use woff2::buffer::Buffer;
use woff2::common::TTC_FONT_FLAVOR;
use woff2::table_tags::{GLYF_TABLE_TAG, KNOWN_TAGS, LOCA_TABLE_TAG};
use woff2::variable_length::{read_255_ushort, read_base128};

fn print_tag(out: &mut Vec<u8>, tag: u32) {
    if tag & 0x8080_8080 != 0 {
        out.extend_from_slice(b"_xfm"); // print _xfm for xform tables (else garbage)
        return;
    }
    // Printed through "%s", so stops at the first NUL byte.
    for b in tag.to_be_bytes() {
        if b == 0 {
            break;
        }
        out.push(b);
    }
}

/// Runs the tool, appending everything it prints to `out`. Returns the exit
/// status.
fn run(out: &mut Vec<u8>, filename: &[u8]) -> u8 {
    macro_rules! p {
        ($($arg:tt)*) => { let _ = write!(out, $($arg)*); };
    }
    macro_rules! r {
        ($e:expr) => {
            match $e {
                Some(v) => v,
                None => return 1,
            }
        };
    }

    let outfilename = util::replace_extension(filename, b".woff2");
    out.extend_from_slice(b"Processing ");
    out.extend_from_slice(filename);
    out.extend_from_slice(b" => ");
    out.extend_from_slice(&outfilename);
    out.extend_from_slice(b"\n");
    let input = util::get_file_content(filename);

    let mut file = Buffer::new(&input);

    p!("WOFF2Header\n");
    let signature = r!(file.read_u32());
    let flavor = r!(file.read_u32());
    let length = r!(file.read_u32());
    let num_tables = r!(file.read_u16());
    let reserved = r!(file.read_u16());
    let total_sfnt_size = r!(file.read_u32());
    let total_compressed_size = r!(file.read_u32());
    let major = r!(file.read_u16());
    let minor = r!(file.read_u16());
    let meta_offset = r!(file.read_u32());
    let meta_length = r!(file.read_u32());
    let meta_orig_length = r!(file.read_u32());
    let priv_offset = r!(file.read_u32());
    let priv_length = r!(file.read_u32());

    if signature != 0x774F_4632 {
        p!("Invalid signature: {:08x}\n", signature);
        return 1;
    }
    p!("signature           0x{:08x}\n", signature);
    p!("flavor              0x{:08x}\n", flavor);
    p!("length              {}\n", length as i32);
    p!("numTables           {}\n", num_tables);
    p!("reserved            {}\n", reserved);
    p!("totalSfntSize       {}\n", total_sfnt_size as i32);
    p!("totalCompressedSize {}\n", total_compressed_size as i32);
    p!("majorVersion        {}\n", major);
    p!("minorVersion        {}\n", minor);
    p!("metaOffset          {}\n", meta_offset as i32);
    p!("metaLength          {}\n", meta_length as i32);
    p!("metaOrigLength      {}\n", meta_orig_length as i32);
    p!("privOffset          {}\n", priv_offset as i32);
    p!("privLength          {}\n", priv_length as i32);

    let mut table_tags: Vec<u32> = Vec::new();
    p!("TableDirectory starts at +{}\n", file.offset());
    p!("Entry offset flags tag  origLength txLength\n");
    for i in 0..i32::from(num_tables) {
        let offset = file.offset();
        let flags = r!(file.read_u8());
        let tag = if (flags & 0x3f) == 0x3f {
            r!(file.read_u32())
        } else {
            KNOWN_TAGS[usize::from(flags & 0x3f)]
        };
        table_tags.push(tag);
        let orig_length = r!(read_base128(&mut file));

        p!("{:5} {:6}  0x{:02x} ", i, offset, flags);
        print_tag(out, tag);
        p!(" {:10}", orig_length as i32);

        let xform_version = (flags >> 6) & 0x3;
        if tag == GLYF_TABLE_TAG || tag == LOCA_TABLE_TAG {
            if xform_version == 0 {
                let transform_length = r!(read_base128(&mut file));
                p!(" {:8}", transform_length as i32);
            }
        } else if xform_version > 0 {
            let transform_length = r!(read_base128(&mut file));
            p!(" {:8}", transform_length as i32);
        }
        p!("\n");
    }

    // Collection header
    if flavor == TTC_FONT_FLAVOR {
        let version = r!(file.read_u32());
        let num_fonts = r!(read_255_ushort(&mut file));
        p!(
            "CollectionHeader 0x{:08x} {} fonts\n",
            version,
            num_fonts as i32
        );

        for i in 0..num_fonts {
            let num_tables = r!(read_255_ushort(&mut file));
            let flavor = r!(file.read_u32());
            p!(
                "CollectionFontEntry {} flavor 0x{:08x} {} tables\n",
                i as i32,
                flavor,
                num_tables as i32
            );
            for j in 0..num_tables {
                let table_idx = r!(read_255_ushort(&mut file));
                if table_idx as usize >= table_tags.len() {
                    return 1;
                }
                p!("  {} ", j as i32);
                print_tag(out, table_tags[table_idx as usize]);
                p!(" (idx {})\n", table_idx as i32);
            }
        }
    }

    p!("TableDirectory ends at +{}\n", file.offset());

    0
}

fn main() -> ExitCode {
    let args: Vec<Vec<u8>> = util::args();
    if args.len() != 2 {
        eprintln!("One argument, the input filename, must be provided.");
        return ExitCode::from(1);
    }
    let mut out = Vec::new();
    let status = run(&mut out, &args[1]);
    util::Stdout::new().print(&out);
    ExitCode::from(status)
}
