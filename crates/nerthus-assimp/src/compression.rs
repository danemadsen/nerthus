//! Assimp's `Compression` (zlib) as importers use it.

use flate2::{Decompress, FlushDecompress, Status};

use crate::scene::{ImportError, fail};

/// `Compression::decompress` with `NoFlush` over a raw deflate stream
/// (`-MaxWBits`): inflated a block at a time until the stream's end; a
/// stream the data ends inside, or a damaged one, fails. Data after the
/// stream's end is ignored; no data gives nothing.
pub(crate) fn inflate_raw(data: &[u8]) -> Result<Vec<u8>, ImportError> {
    if data.is_empty() {
        return Ok(Vec::new());
    }
    let mut decompress = Decompress::new(false);
    let mut output = Vec::new();
    loop {
        let consumed = decompress.total_in() as usize;
        let produced = decompress.total_out();
        output.reserve(32786);
        let status = decompress.decompress_vec(&data[consumed..], &mut output, FlushDecompress::None);
        match status {
            Ok(Status::StreamEnd) => return Ok(output),
            Ok(Status::Ok) if decompress.total_in() as usize != consumed || decompress.total_out() != produced => {}
            _ => return fail("Compression: Failure decompressing this file using gzip."),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn whole_streams_inflate_and_cut_ones_fail() {
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(b"<WORLD></WORLD>").unwrap();
        let mut stream = encoder.finish().unwrap();
        assert_eq!(inflate_raw(&stream).unwrap(), b"<WORLD></WORLD>");
        stream.extend_from_slice(b"trailing");
        assert_eq!(inflate_raw(&stream).unwrap(), b"<WORLD></WORLD>");
        assert!(inflate_raw(&stream[..4]).is_err());
    }
}
