//! A very simple commandline tool for decompressing woff2 format files to
//! true type font files.

#[path = "util.rs"]
mod util;

use std::process::ExitCode;

use woff2::Woff2Out;

fn main() -> ExitCode {
    let args: Vec<Vec<u8>> = util::args();
    if args.len() != 2 {
        eprintln!("One argument, the input filename, must be provided.");
        return ExitCode::from(1);
    }

    let filename = &args[1];
    let outfilename = util::replace_extension(filename, b".ttf");

    let input = util::get_file_content(filename);
    let initial = woff2::compute_woff2_final_size(&input).min(woff2::DEFAULT_MAX_SIZE);
    let mut output = vec![0u8; initial];
    let (ok, size) = {
        let mut out = woff2::StringOut::new(&mut output);
        let ok = woff2::convert_woff2_to_ttf(&input, &mut out);
        (ok, out.size())
    };

    if ok {
        util::set_file_contents(&outfilename, &output[..size]);
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
