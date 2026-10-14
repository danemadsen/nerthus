//! Reads each image named on the command line as the File node does
//! (`read_pixel_cache`) and writes what it holds beside the image:
//! `<file>.ours.raw` (the cache's quanta, interleaved little-endian f32)
//! and `<file>.ours.json` (size, channels, colorspace and gamma, or the
//! error), for comparing with ImageMagick's own.

use std::path::Path;

fn main() {
    for argument in std::env::args().skip(1) {
        let path = Path::new(&argument);
        let name = path.file_name().map(|name| name.to_string_lossy().to_string()).unwrap_or_default();
        let json_path = path.with_file_name(format!("{name}.ours.json"));
        let raw_path = path.with_file_name(format!("{name}.ours.raw"));
        let read = std::panic::catch_unwind(|| nerthus_io::pixel_cache::read_pixel_cache(path));
        let json = match read {
            Ok(Ok(cache)) => {
                let bytes: Vec<u8> = cache.data.iter().flat_map(|value| value.to_le_bytes()).collect();
                std::fs::write(&raw_path, bytes).expect("the dump is written");
                let channels: Vec<String> = cache.channels.iter().map(|channel| format!("\"{channel:?}\"")).collect();
                format!(
                    "{{\"width\":{},\"height\":{},\"channels\":[{}],\"colorspace\":\"{:?}\",\"gamma\":{}}}",
                    cache.width,
                    cache.height,
                    channels.join(","),
                    cache.colorspace,
                    cache.gamma
                )
            }
            Ok(Err(error)) => {
                let _ = std::fs::remove_file(&raw_path);
                format!("{{\"error\":{:?}}}", format!("{error:#}"))
            }
            Err(_) => {
                let _ = std::fs::remove_file(&raw_path);
                "{\"error\":\"panic\"}".to_string()
            }
        };
        std::fs::write(&json_path, json).expect("the report is written");
    }
}
