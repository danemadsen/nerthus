//! Decodes a DNG file as ImageMagick (LibRaw) does and writes the
//! 16-bit samples in tools/re/libraw-probe's final.bin layout (u32 width,
//! height, colours, bits, then the samples) for comparing with it.
//!
//! usage: dng-decode <file.dng> <out.bin>

use std::io::Write;

fn main() -> anyhow::Result<()> {
    let arguments: Vec<String> = std::env::args().collect();
    if arguments.len() < 3 {
        anyhow::bail!("usage: dng-decode <file.dng> <out.bin>");
    }
    let start = std::time::Instant::now();
    let cache = nerthus_io::coders::dng::read(std::path::Path::new(&arguments[1]))?;
    println!("{}x{} {} channels in {:?}", cache.width, cache.height, cache.channels.len(), start.elapsed());
    let mut output = std::io::BufWriter::new(std::fs::File::create(&arguments[2])?);
    let header = [cache.width as u32, cache.height as u32, cache.channels.len() as u32, 16];
    for value in header {
        output.write_all(&value.to_le_bytes())?;
    }
    for &sample in &cache.data {
        output.write_all(&(sample as u16).to_le_bytes())?;
    }
    Ok(())
}
