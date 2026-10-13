//! `ObjFileMtlImporter`: what of a material library reaches the meshes is
//! which material is current afterwards (and the materials it adds), and
//! whether a number fails to parse.

use super::parser::{DEFAULT_MATERIAL, Model};
use super::tools::{at, copy_next_word, get_float, get_name, is_end_of_buffer, is_line_end, next_token, skip_line, starts_with_ignore_case, trim};
use crate::fast_atof::fast_atof;
use crate::scene::{ImportError, fail};

/// `ObjFileMtlImporter::BUFFERSIZE`.
const WORD_BUFFER_SIZE: usize = 2048;

/// `BaseImporter::TextFileToBuffer(…, ALLOW_EMPTY)`: `ConvertToUTF8`
/// (which needs 8 bytes, drops a UTF-8 BOM and byte-swaps big-endian
/// UTF-16/32 but keeps the result in UTF-16/32) and a closing NUL.
pub fn text_file_to_buffer(mut data: Vec<u8>) -> Result<Vec<u8>, ImportError> {
    if !data.is_empty() {
        if data.len() < 8 {
            return fail("File is too small");
        }
        if data.starts_with(&[0xEF, 0xBB, 0xBF]) {
            data.drain(..3);
        } else {
            if data.starts_with(&[0x00, 0x00, 0xFE, 0xFF]) {
                if !data.len().is_multiple_of(4) {
                    return fail("OBJ: unaligned UTF-32 material library");
                }
                for word in data.as_chunks_mut::<4>().0 {
                    word.reverse();
                }
            }
            // A "UTF-32 LE" mark (as it reads FE FF 00 00) ends the
            // conversion before the UTF-16 one.
            let utf32_mark = data.starts_with(&[0xFE, 0xFF, 0x00, 0x00]);
            if !utf32_mark && data.starts_with(&[0xFE, 0xFF]) && data.len().is_multiple_of(2) {
                for pair in data.as_chunks_mut::<2>().0 {
                    pair.swap(0, 1);
                }
            }
        }
    }
    data.push(0);
    Ok(data)
}

pub struct MtlImporter<'a> {
    buffer: Vec<u8>,
    it: usize,
    model: &'a mut Model,
}

impl<'a> MtlImporter<'a> {
    pub fn new(buffer: Vec<u8>, model: &'a mut Model) -> Self {
        MtlImporter { buffer, it: 0, model }
    }

    fn end(&self) -> usize {
        self.buffer.len()
    }

    fn byte(&self) -> u8 {
        at(&self.buffer, self.it)
    }

    fn skip_line(&mut self) {
        self.it = skip_line(&self.buffer, self.it, self.end());
    }

    fn has_material(&self) -> bool {
        self.model.current_material.is_some()
    }

    /// `load`.
    pub fn load(mut self) -> Result<(), ImportError> {
        while self.it != self.end() {
            match self.byte() {
                b'k' | b'K' => {
                    self.it += 1;
                    if matches!(self.byte(), b'a' | b'd' | b's' | b'e') {
                        self.it += 1;
                        if self.has_material() {
                            self.get_color()?;
                        }
                    }
                    self.skip_line();
                }
                b'T' => {
                    self.it += 1;
                    if self.byte() == b'f' {
                        self.it += 1;
                        if self.has_material() {
                            self.get_color()?;
                        }
                    } else if self.byte() == b'r' {
                        self.it += 1;
                        self.get_float_value()?;
                    }
                    self.skip_line();
                }
                b'd' => {
                    let next = [at(&self.buffer, self.it + 1), at(&self.buffer, self.it + 2), at(&self.buffer, self.it + 3)];
                    if next == *b"isp" {
                        self.get_texture()?;
                    } else {
                        self.it += 1;
                        if self.has_material() {
                            self.get_float_value()?;
                        }
                        self.skip_line();
                    }
                }
                b'N' | b'n' => {
                    self.it += 1;
                    match self.byte() {
                        b's' | b'i' => {
                            self.it += 1;
                            if self.has_material() {
                                self.get_float_value()?;
                            }
                        }
                        b'e' => self.create_material(),
                        b'o' => {
                            self.it -= 1;
                            self.get_texture()?;
                        }
                        _ => {}
                    }
                    self.skip_line();
                }
                b'P' => {
                    self.it += 1;
                    match self.byte() {
                        b'r' | b'm' => {
                            self.it += 1;
                            if self.has_material() {
                                self.get_float_value()?;
                            }
                        }
                        b's' => {
                            self.it += 1;
                            if self.has_material() {
                                self.get_color()?;
                            }
                        }
                        b'c' => {
                            self.it += 1;
                            if self.byte() == b'r' {
                                self.it += 1;
                            }
                            if self.has_material() {
                                self.get_float_value()?;
                            }
                        }
                        _ => {}
                    }
                    self.skip_line();
                }
                b'm' | b'b' | b'r' => {
                    self.get_texture()?;
                    self.skip_line();
                }
                b'i' => {
                    self.it = next_token(&self.buffer, self.it, self.end());
                    if self.has_material() {
                        let (it, _) = copy_next_word(&self.buffer, self.it, self.end(), WORD_BUFFER_SIZE);
                        self.it = it;
                    }
                    self.skip_line();
                }
                b'a' => {
                    self.it += 1;
                    if self.has_material() {
                        self.get_float_value()?;
                    }
                    self.skip_line();
                }
                _ => self.skip_line(),
            }
        }
        Ok(())
    }

    /// `getColorRGBA`: one value, or three when more follows on the line.
    fn get_color(&mut self) -> Result<(), ImportError> {
        let end = self.end();
        let (it, _) = get_float(&self.buffer, self.it, end)?;
        self.it = it;
        if !is_line_end(self.byte()) {
            let (it, _) = get_float(&self.buffer, self.it, end)?;
            let (it, _) = get_float(&self.buffer, it, end)?;
            self.it = it;
        }
        Ok(())
    }

    /// `getFloatValue`: an empty word is no value; anything else must be a
    /// number.
    fn get_float_value(&mut self) -> Result<(), ImportError> {
        let (it, word) = copy_next_word(&self.buffer, self.it, self.end(), WORD_BUFFER_SIZE);
        self.it = it;
        if !word.is_empty() {
            fast_atof(&word)?;
        }
        Ok(())
    }

    /// `createMaterial` (`newmtl`, read from the 'e' on): the name after
    /// the keyword, trimmed (`DefaultMaterial` when there is none).
    fn create_material(&mut self) {
        let mut line = Vec::new();
        while !is_line_end(self.byte()) {
            line.push(self.byte());
            self.it += 1;
        }
        let is_delimiter = |byte: &u8| *byte == b' ' || *byte == b'\t';
        let token_count = line.split(is_delimiter).filter(|token| !token.is_empty()).count();
        let mut name = Vec::new();
        if token_count == 1 {
            name = DEFAULT_MATERIAL.to_vec();
        } else if let Some(first_space) = line.iter().position(is_delimiter)
            && let Some(offset) = line[first_space..].iter().position(|byte| !is_delimiter(byte))
        {
            name = line[first_space + offset..].to_vec();
        }
        let name = trim(&name);
        match self.model.material_map.get(&name) {
            Some(&material) => self.model.current_material = Some(material),
            None => {
                let material = self.model.new_material(&name);
                self.model.material_lib.push(name.clone());
                self.model.material_map.insert(name, material);
                self.model.current_material = Some(material);
                if let Some(mesh) = self.model.current_mesh {
                    self.model.meshes[mesh].material_index = (self.model.material_lib.len() - 1) as u32;
                }
            }
        }
    }

    /// `getTexture`: a known map keyword, its options, then the name (a
    /// texture with no current material creates "Empty_Material").
    fn get_texture(&mut self) -> Result<(), ImportError> {
        if self.model.current_material.is_none() {
            let material = self.model.new_material(b"Empty_Material");
            self.model.material_map.insert(b"Empty_Material".to_vec(), material);
            self.model.current_material = Some(material);
        }
        let keywords: [&[u8]; 18] = [
            b"map_Kd", b"map_Ka", b"map_Ks", b"map_disp", b"disp", b"map_d", b"map_emissive", b"map_Ke", b"map_bump", b"bump", b"map_Kn", b"norm",
            b"refl", b"map_ns", b"map_Pr", b"map_Pm", b"map_Ps", b"map_Ps",
        ];
        let Some(keyword) = keywords.iter().position(|keyword| starts_with_ignore_case(&self.buffer, self.it, keyword)) else {
            return Ok(());
        };
        if keywords[keyword] == b"refl" {
            return Ok(());
        }
        self.get_texture_option()?;
        let (it, _) = get_name(&self.buffer, self.it, self.end());
        self.it = it;
        Ok(())
    }

    /// `getTextureOption`: steps over "-option value…" groups; "-bm" must
    /// be followed by a number.
    fn get_texture_option(&mut self) -> Result<(), ImportError> {
        let end = self.end();
        self.it = next_token(&self.buffer, self.it, end);
        while !is_end_of_buffer(self.it, end) && self.byte() == b'-' {
            let option = |name: &[u8]| starts_with_ignore_case(&self.buffer, self.it, name);
            let mut skip_tokens = 1;
            if option(b"-clamp") || option(b"-type") {
                skip_tokens = 2;
            } else if option(b"-bm") {
                let value_start = next_token(&self.buffer, self.it, end);
                get_float(&self.buffer, value_start, end)?;
                skip_tokens = 2;
            } else if option(b"-blendu") || option(b"-blendv") || option(b"-boost") || option(b"-texres") || option(b"-imfchan") {
                skip_tokens = 2;
            } else if option(b"-mm") {
                skip_tokens = 3;
            } else if option(b"-o") || option(b"-s") || option(b"-t") {
                skip_tokens = 4;
            }
            for _ in 0..skip_tokens {
                self.it = next_token(&self.buffer, self.it, end);
            }
        }
        Ok(())
    }
}
