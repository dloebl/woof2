//! A commandline tool for compressing ttf format files to woff2.

#[path = "util.rs"]
mod util;

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<Vec<u8>> = util::args();
    if args.len() != 2 {
        eprintln!("One argument, the input filename, must be provided.");
        return ExitCode::from(1);
    }

    let filename = &args[1];
    let outfilename = util::replace_extension(filename, b".woff2");
    let mut stdout = util::Stdout::new();
    let mut line = b"Processing ".to_vec();
    line.extend_from_slice(filename);
    line.extend_from_slice(b" => ");
    line.extend_from_slice(&outfilename);
    line.push(b'\n');
    stdout.print(&line);
    let input = util::get_file_content(filename);

    let params = woff2::Woff2Params::default();
    let mut output_size = woff2::max_woff2_compressed_size(input.len(), 0);
    let mut output = vec![0u8; output_size];
    if !woff2::convert_ttf_to_woff2_with(
        &input,
        &mut output,
        &mut output_size,
        &params,
        woff2::brotli::SYSTEM_COMPRESS,
    ) {
        eprintln!("Compression failed.");
        return ExitCode::from(1);
    }
    output.truncate(output_size);

    util::set_file_contents(&outfilename, &output);

    ExitCode::SUCCESS
}
