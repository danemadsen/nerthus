//! ImageMagick's PSD and PSB reader (`coders/psd.c`), as the File node
//! meets it through Magick.NET, which reads a single image: the merged
//! (composite) image when the file says it has one, else the layers read
//! and flattened onto a transparent canvas with their blend modes,
//! opacities and masks.
//!
//! What IM leaves in the pixel cache follows the file's mode: grey, sRGB,
//! CMYK (negated) or Lab, with the merged image's extra channels as meta
//! channels (an alpha channel IM does not know it has is dropped).
//! Grayscale, bitmap, indexed and duotone images (below 32 bits) get a
//! colormap, which stops IM from reading their merged image: such a file
//! reads only through its layers, and one without layers does not read.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::pixel_cache::{CacheColorspace, PixelCache, PixelChannel, char_quantum};

/// `MaxPSDChannels`.
const MAX_PSD_CHANNELS: u16 = 56;
/// `MaxPixelChannels`, and where meta channels start (`MetaPixelChannels`).
const MAX_PIXEL_CHANNELS: i64 = 64;
const META_PIXEL_CHANNELS: i64 = 10;
const QUANTUM_RANGE: f64 = 65535.0;
const QUANTUM_SCALE: f64 = 1.0 / 65535.0;
const MAGICK_EPSILON: f64 = 1.0e-12;
/// IM's default image gamma (`1.000/2.200`).
const IMAGE_GAMMA: f64 = 1.0 / 2.2;
/// The largest side IM is allowed to make a mask image (`CloneImage`
/// beyond it fails, as for a mask whose rectangle is upside down).
const MASK_LIMIT: u64 = 300000;

const BITMAP_MODE: u16 = 0;
const GRAYSCALE_MODE: u16 = 1;
const INDEXED_MODE: u16 = 2;
const RGB_MODE: u16 = 3;
const CMYK_MODE: u16 = 4;
const MULTICHANNEL_MODE: u16 = 7;
const DUOTONE_MODE: u16 = 8;
const LAB_MODE: u16 = 9;

const RAW: u16 = 0;
const RLE: u16 = 1;
const ZIP_WITHOUT_PREDICTION: u16 = 2;
const ZIP_WITH_PREDICTION: u16 = 3;

/// Reads a PSD or PSB file as Magick.NET does.
pub fn read(path: &Path) -> Result<PixelCache> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot open {}", path.display()))?;
    let image = decode(&bytes)?;
    Ok(image.into_pixel_cache())
}

/// The colorspaces the reader gives images.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Space {
    Gray,
    SRgb,
    Cmyk,
    Lab,
}

/// IM's pixel channels (`PixelChannel`), as far as PSD channels reach them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Channel {
    Red,
    Green,
    Blue,
    Black,
    Alpha,
    Index,
    ReadMask,
    WriteMask,
    CompositeMask,
    /// The deprecated `MetaPixelChannel` (8) and the unused 9.
    Unused,
    Meta(usize),
}

impl Channel {
    /// `(PixelChannel) index`.
    fn from_number(number: i64) -> Channel {
        match number {
            0 => Channel::Red,
            1 => Channel::Green,
            2 => Channel::Blue,
            3 => Channel::Black,
            4 => Channel::Alpha,
            5 => Channel::Index,
            6 => Channel::ReadMask,
            7 => Channel::WriteMask,
            9 => Channel::CompositeMask,
            number if number >= META_PIXEL_CHANNELS => Channel::Meta((number - META_PIXEL_CHANNELS) as usize),
            _ => Channel::Unused,
        }
    }
}

/// `CompositeOperator`s a PSD layer can have (`PSDBlendModeToCompositeOperator`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compose {
    Over,
    Multiply,
    Dissolve,
    Difference,
    Darken,
    Lighten,
    Hue,
    Saturate,
    Colorize,
    Luminize,
    Screen,
    Overlay,
    HardLight,
    SoftLight,
    Exclusion,
    ColorDodge,
    ColorBurn,
    LinearBurn,
    LinearDodge,
    LinearLight,
    VividLight,
    PinLight,
    HardMix,
    /// A hidden layer (`NoCompositeOp`).
    No,
}

impl Compose {
    fn from_blend_key(key: &[u8]) -> Compose {
        match key {
            b"norm" => Compose::Over,
            b"mul " => Compose::Multiply,
            b"diss" => Compose::Dissolve,
            b"diff" => Compose::Difference,
            b"dark" => Compose::Darken,
            b"lite" => Compose::Lighten,
            b"hue " => Compose::Hue,
            b"sat " => Compose::Saturate,
            b"colr" => Compose::Colorize,
            b"lum " => Compose::Luminize,
            b"scrn" => Compose::Screen,
            b"over" => Compose::Overlay,
            b"hLit" => Compose::HardLight,
            b"sLit" => Compose::SoftLight,
            b"smud" => Compose::Exclusion,
            b"div " => Compose::ColorDodge,
            b"idiv" => Compose::ColorBurn,
            b"lbrn" => Compose::LinearBurn,
            b"lddg" => Compose::LinearDodge,
            b"lLit" => Compose::LinearLight,
            b"vLit" => Compose::VividLight,
            b"pLit" => Compose::PinLight,
            b"hMix" => Compose::HardMix,
            _ => Compose::Over,
        }
    }
}

/// An IM `Image` as the reader builds it: its pixel cache (channels laid
/// out as `ResetPixelChannelMap` lays them) and what goes with it.
#[derive(Clone, Debug)]
struct Picture {
    width: usize,
    height: usize,
    depth: u16,
    space: Space,
    /// `alpha_trait` is `BlendPixelTrait`.
    alpha: bool,
    /// `PseudoClass`: the image has a colormap.
    pseudo: bool,
    meta: usize,
    data: Vec<f32>,
    /// `page.x`, `page.y`.
    page: (i64, i64),
    compose: Compose,
}

impl Picture {
    /// `AcquireImage` and `SetImageExtent` with pixels reset to zero.
    fn new(width: usize, height: usize) -> Picture {
        let mut picture = Picture { width, height, depth: 8, space: Space::SRgb, alpha: false, pseudo: false, meta: 0, data: Vec::new(), page: (0, 0), compose: Compose::Over };
        picture.data = vec![0.0; width * height * picture.stride()];
        picture
    }

    fn colour_count(&self) -> usize {
        match self.space {
            Space::Gray => 1,
            Space::SRgb | Space::Lab => 3,
            Space::Cmyk => 4,
        }
    }

    /// The channels at each offset (`channel_map[offset].channel`).
    fn layout(&self) -> Vec<Channel> {
        let mut layout = match self.space {
            Space::Gray => vec![Channel::Red],
            Space::SRgb | Space::Lab => vec![Channel::Red, Channel::Green, Channel::Blue],
            Space::Cmyk => vec![Channel::Red, Channel::Green, Channel::Blue, Channel::Black],
        };
        if self.alpha {
            layout.push(Channel::Alpha);
        }
        if self.pseudo {
            layout.push(Channel::Index);
        }
        for index in 0..self.meta {
            layout.push(Channel::Meta(index));
        }
        layout
    }

    fn stride(&self) -> usize {
        self.layout().len()
    }

    /// `channel_map[channel].offset`, for a channel the image has (a grey
    /// image's green and blue are its grey).
    fn offset(&self, channel: Channel) -> Option<usize> {
        let colours = self.colour_count();
        match channel {
            Channel::Red => Some(0),
            Channel::Green | Channel::Blue if self.space == Space::Gray => Some(0),
            Channel::Green => Some(1),
            Channel::Blue => Some(2),
            Channel::Black if self.space == Space::Cmyk => Some(3),
            Channel::Alpha if self.alpha => Some(colours),
            Channel::Index if self.pseudo => Some(colours + usize::from(self.alpha)),
            Channel::Meta(index) if index < self.meta => Some(colours + usize::from(self.alpha) + usize::from(self.pseudo) + index),
            _ => None,
        }
    }

    /// The cache synced to new settings (`SyncImagePixelCache` after a
    /// change of colorspace, alpha, class or meta channels): each channel
    /// the old cache had is copied, new ones start at zero.
    fn relayout(&mut self, change: impl FnOnce(&mut Picture)) {
        let old = Picture { data: Vec::new(), ..self.clone() };
        let old_stride = old.stride();
        change(self);
        let layout = self.layout();
        let sources: Vec<Option<usize>> = layout.iter().map(|channel| old.offset(*channel)).collect();
        let mut data = Vec::with_capacity(self.width * self.height * layout.len());
        for pixel in self.data.chunks_exact(old_stride.max(1)) {
            for source in &sources {
                data.push(source.map_or(0.0, |offset| pixel[offset]));
            }
        }
        self.data = data;
    }

    fn set_space(&mut self, space: Space) {
        self.relayout(|picture| picture.space = space);
    }

    fn set_alpha(&mut self, alpha: bool) {
        self.relayout(|picture| picture.alpha = alpha);
    }

    /// `SetPixelChannel` on one pixel: nothing for a channel the image
    /// does not have.
    fn set(&mut self, pixel: usize, channel: Channel, value: f32) {
        if let Some(offset) = self.offset(channel) {
            let stride = self.stride();
            self.data[pixel * stride + offset] = value;
        }
    }

    /// `SetImageBackgroundColor` with IM's white background (transparent
    /// when asked): the image made `DirectClass`, alpha switched on for a
    /// transparent background, every pixel filled (`ConformPixelInfo` takes
    /// white to CMYK as no ink).
    fn fill_background(&mut self, transparent: bool) {
        self.relayout(|picture| {
            picture.pseudo = false;
            if transparent {
                picture.alpha = true;
            }
        });
        let colour = if self.space == Space::Cmyk { 0.0 } else { QUANTUM_RANGE as f32 };
        let alpha = if transparent { 0.0 } else { QUANTUM_RANGE as f32 };
        let layout = self.layout();
        let stride = layout.len();
        for pixel in self.data.chunks_exact_mut(stride) {
            for (value, channel) in pixel.iter_mut().zip(layout.iter()) {
                match channel {
                    Channel::Red | Channel::Green | Channel::Blue | Channel::Black => *value = colour,
                    Channel::Alpha => *value = alpha,
                    _ => {}
                }
            }
        }
    }

    /// The pixel cache IM leaves for the File node.
    fn into_pixel_cache(self) -> PixelCache {
        let gray = self.space == Space::Gray;
        let channels: Vec<PixelChannel> = self
            .layout()
            .into_iter()
            .map(|channel| match channel {
                Channel::Red if gray => PixelChannel::Gray,
                Channel::Red => PixelChannel::Red,
                Channel::Green => PixelChannel::Green,
                Channel::Blue => PixelChannel::Blue,
                Channel::Black => PixelChannel::Black,
                Channel::Alpha => PixelChannel::Alpha,
                Channel::Index => PixelChannel::Index,
                _ => PixelChannel::Meta,
            })
            .collect();
        let colorspace = match self.space {
            Space::Gray => CacheColorspace::Gray,
            Space::SRgb => CacheColorspace::SRgb,
            Space::Cmyk => CacheColorspace::Cmyk,
            Space::Lab => CacheColorspace::Lab,
        };
        PixelCache::new(self.width, self.height, channels, self.data, colorspace, IMAGE_GAMMA)
    }
}

/// `PSDInfo`.
#[derive(Clone, Debug)]
struct PsdInfo {
    version: u16,
    channels: u16,
    mode: u16,
    min_channels: u16,
    has_merged_image: bool,
}

/// A layer as its record describes it (`LayerInfo`).
#[derive(Clone, Debug, Default)]
struct LayerInfo {
    /// Each channel's pixel channel (none when IM does not read it) and
    /// data size.
    channels: Vec<(Option<Channel>, u64)>,
    blend_key: Vec<u8>,
    opacity: f32,
    visible: bool,
    left: i64,
    top: i64,
    width: u64,
    height: u64,
    mask: MaskInfo,
}

/// A layer's mask (`MaskInfo`).
#[derive(Clone, Debug, Default)]
struct MaskInfo {
    left: i64,
    top: i64,
    width: u64,
    height: u64,
    background: u8,
    flags: u8,
}

/// The file as a blob that IM has mapped into memory (`BlobStream`):
/// reads past the end come up short and set end of file, which a seek
/// back into the file clears.
struct Blob<'a> {
    bytes: &'a [u8],
    offset: usize,
    eof: bool,
}

impl<'a> Blob<'a> {
    fn new(bytes: &'a [u8]) -> Blob<'a> {
        Blob { bytes, offset: 0, eof: false }
    }

    fn size(&self) -> u64 {
        self.bytes.len() as u64
    }

    /// `ReadBlob`: up to `length` bytes.
    fn read(&mut self, length: usize) -> &'a [u8] {
        if self.offset >= self.bytes.len() {
            self.eof = true;
            return &[];
        }
        let end = self.offset.saturating_add(length).min(self.bytes.len());
        let read = &self.bytes[self.offset..end];
        self.offset = end;
        if read.len() != length {
            self.eof = true;
        }
        read
    }

    /// `ReadBlobByte`: none at the end.
    fn byte(&mut self) -> Option<u8> {
        if self.offset >= self.bytes.len() {
            self.eof = true;
            return None;
        }
        let byte = self.bytes[self.offset];
        self.offset += 1;
        Some(byte)
    }

    /// `(unsigned char) ReadBlobByte(image)`: 255 at the end.
    fn byte_or_eof(&mut self) -> u8 {
        self.byte().unwrap_or(255)
    }

    fn u16(&mut self) -> u16 {
        let bytes = self.read(2);
        if bytes.len() != 2 {
            return 0;
        }
        u16::from_be_bytes([bytes[0], bytes[1]])
    }

    fn u32(&mut self) -> u32 {
        let bytes = self.read(4);
        if bytes.len() != 4 {
            return 0;
        }
        u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
    }

    fn u64(&mut self) -> u64 {
        let bytes = self.read(8);
        if bytes.len() != 8 {
            return 0;
        }
        let mut array = [0u8; 8];
        array.copy_from_slice(bytes);
        u64::from_be_bytes(array)
    }

    fn i16(&mut self) -> i16 {
        self.u16() as i16
    }

    fn i32(&mut self) -> i32 {
        self.u32() as i32
    }

    /// `GetPSDSize`: 32 bits in a PSD, 64 in a PSB.
    fn psd_size(&mut self, version: u16) -> u64 {
        if version == 1 {
            return self.u32() as u64;
        }
        self.u64()
    }

    /// `SeekBlob(SEEK_SET)`.
    fn seek(&mut self, offset: u64) {
        self.offset = usize::try_from(offset).unwrap_or(usize::MAX);
        if self.offset < self.bytes.len() {
            self.eof = false;
        }
    }

    /// `SeekBlob(SEEK_CUR)`.
    fn skip(&mut self, length: u64) {
        let offset = (self.offset as u64).saturating_add(length);
        self.seek(offset);
    }

    /// `SeekBlob(SEEK_CUR)` by an offset that may go back (a position
    /// before the start fails and leaves the blob where it was).
    fn skip_signed(&mut self, offset: i64) {
        let target = self.offset as i64 + offset;
        if target < 0 {
            return;
        }
        self.seek(target as u64);
    }

    fn tell(&self) -> u64 {
        self.offset as u64
    }

    /// `DiscardBlobBytes`: false when the file ends first.
    fn discard(&mut self, length: u64) -> bool {
        if length > i64::MAX as u64 {
            return false;
        }
        let available = self.bytes.len().saturating_sub(self.offset) as u64;
        let taken = length.min(available);
        self.read(taken as usize);
        taken == length
    }
}

/// `ReadPSDImage`.
fn decode(bytes: &[u8]) -> Result<Picture> {
    let mut blob = Blob::new(bytes);
    let signature = blob.read(4);
    let version = blob.u16();
    if signature != b"8BPS" || (version != 1 && version != 2) {
        bail!("ImproperImageHeader");
    }
    blob.read(6);
    let channels = blob.u16();
    if channels < 1 {
        bail!("MissingImageChannel");
    }
    if channels > MAX_PSD_CHANNELS {
        bail!("MaximumChannelsExceeded");
    }
    let rows = blob.u32();
    let columns = blob.u32();
    let limit = if version == 1 { 30000 } else { 300000 };
    if rows > limit || columns > limit {
        bail!("ImproperImageHeader");
    }
    let depth = blob.u16();
    if !matches!(depth, 1 | 8 | 16 | 32) {
        bail!("ImproperImageHeader");
    }
    let mode = blob.u16();
    if mode == INDEXED_MODE && channels > 3 {
        bail!("ImproperImageHeader");
    }
    if blob.eof {
        bail!("ImproperImageHeader");
    }
    if rows == 0 || columns == 0 {
        bail!("NegativeOrZeroImageSize");
    }
    let mut image = Picture::new(columns as usize, rows as usize);
    image.depth = depth;
    let mut info = PsdInfo { version, channels, mode, min_channels: 3, has_merged_image: true };
    match mode {
        LAB_MODE => image.set_space(Space::Lab),
        CMYK_MODE => {
            info.min_channels = 4;
            image.set_space(Space::Cmyk);
        }
        BITMAP_MODE | GRAYSCALE_MODE | DUOTONE_MODE => {
            if depth != 32 {
                image.relayout(|picture| picture.pseudo = true);
            }
            info.min_channels = 1;
            image.set_space(Space::Gray);
        }
        INDEXED_MODE => info.min_channels = 1,
        MULTICHANNEL_MODE if channels < 3 => {
            info.min_channels = channels;
            image.set_space(Space::Gray);
        }
        _ => {}
    }
    if channels < info.min_channels {
        bail!("ImproperImageHeader");
    }

    // The colormap, only in indexed and duotone images.
    let length = blob.u32() as u64;
    if mode == INDEXED_MODE && length < 3 {
        bail!("ImproperImageHeader");
    }
    if length != 0 {
        if mode == DUOTONE_MODE || depth == 32 {
            blob.skip(length);
        } else {
            let colours = length / 3;
            if colours > 65536 {
                bail!("ImproperImageHeader");
            }
            // The colormap's values never reach the pixels: a colormapped
            // image's merged image is not read.
            let entries = colours.max(1);
            for _ in 0..entries * 3 {
                blob.byte();
            }
            image.relayout(|picture| {
                picture.pseudo = true;
                picture.alpha = false;
            });
        }
    }
    if depth == 1 && !image.pseudo {
        bail!("ImproperImageHeader");
    }

    // Image resource blocks: whether the merged image is real.
    let length = blob.u32() as u64;
    if length != 0 {
        if length > blob.size() {
            bail!("InsufficientImageDataInFile");
        }
        let blocks = blob.read(length as usize);
        if blocks.len() as u64 != length || length < 4 || &blocks[..4] != b"8BIM" {
            bail!("ImproperImageHeader");
        }
        parse_resource_blocks(&mut info, blocks);
    }

    // Layer and mask information.
    let mut length = blob.psd_size(version);
    if length == 8 {
        blob.u32();
        length = blob.u32() as u64;
    }
    let offset = blob.tell();
    let skip_layers = info.has_merged_image;
    let mut layers: Vec<Picture> = Vec::new();
    if length != 0 {
        read_layers(&mut blob, &mut image, &info, skip_layers, &mut layers)?;
        blob.seek(offset.saturating_add(length));
    }
    if blob.eof {
        bail!("UnexpectedEndOfFile");
    }

    // The precombined image.
    let mut list_length = 1 + layers.len();
    if info.has_merged_image || list_length == 1 {
        info.has_merged_image = read_merged_image(&mut blob, &mut image, &info)?;
    }
    if !info.has_merged_image && list_length == 1 && length != 0 {
        blob.seek(offset);
        read_layers(&mut blob, &mut image, &info, false, &mut layers)?;
        list_length = 1 + layers.len();
    }
    if info.has_merged_image {
        return Ok(image);
    }
    if list_length == 1 {
        bail!("InsufficientImageDataInFile");
    }
    image.fill_background(true);
    Ok(flatten(image, layers))
}

/// `ParseImageResourceBlocks`: only `has_merged_image` (block 0x0421, the
/// version information) matters to the pixels.
fn parse_resource_blocks(info: &mut PsdInfo, blocks: &[u8]) {
    let length = blocks.len();
    if length < 16 {
        return;
    }
    let short = |at: usize| u16::from_be_bytes([blocks[at], blocks[at + 1]]);
    let long = |at: usize| u32::from_be_bytes([blocks[at], blocks[at + 1], blocks[at + 2], blocks[at + 3]]);
    let mut p = 0usize;
    while p < length - 7 {
        if &blocks[p..p + 4] != b"8BIM" {
            break;
        }
        p += 4;
        let id = short(p);
        p += 2;
        let mut name_length = blocks[p] as usize;
        p += 1;
        if name_length.is_multiple_of(2) {
            name_length += 1;
        }
        p += name_length;
        if p > length - 4 {
            break;
        }
        let count = long(p) as usize;
        p += 4;
        if p + count > length {
            break;
        }
        match id {
            // Resolution: eight shorts, unless the block is too short (when
            // the block is not skipped either).
            0x03ed => {
                if count >= 16 {
                    p += 16;
                }
            }
            0x0421 => {
                if count > 4 && blocks[p + 4] == 0 {
                    info.has_merged_image = false;
                }
                p += count;
            }
            _ => p += count,
        }
        if count & 1 != 0 {
            p += 1;
        }
    }
}

/// `GetPixelChannelFromPSDIndex`: none when IM does not read the channel.
fn channel_from_index(mode: u16, index: i64) -> Option<Channel> {
    let mut index = index;
    match mode {
        BITMAP_MODE | INDEXED_MODE | GRAYSCALE_MODE => {
            if index == 1 {
                index = -1;
            } else if index > 1 {
                index = META_PIXEL_CHANNELS + index - 2;
            }
        }
        LAB_MODE | MULTICHANNEL_MODE | RGB_MODE => {
            if index == 3 {
                index = -1;
            } else if index > 3 {
                index = META_PIXEL_CHANNELS + index - 4;
            }
        }
        CMYK_MODE => {
            if index == 4 {
                index = -1;
            } else if index > 4 {
                index = META_PIXEL_CHANNELS + index - 5;
            }
        }
        _ => {}
    }
    if !(-2..MAX_PIXEL_CHANNELS).contains(&index) {
        return None;
    }
    match index {
        -1 => Some(Channel::Alpha),
        -2 => Some(Channel::ReadMask),
        index => Some(Channel::from_number(index)),
    }
}

/// `SetPSDMetaChannels`: the channels beyond the mode's and alpha become
/// meta channels. False for a colormapped image, which IM then leaves
/// unread.
fn set_meta_channels(image: &mut Picture, info: &PsdInfo, channels: u16) -> bool {
    if image.pseudo {
        return false;
    }
    let mut meta = channels as i64 - info.min_channels as i64;
    if image.alpha {
        meta -= 1;
    }
    if meta > 0 {
        let meta = meta as usize;
        if meta >= (MAX_PIXEL_CHANNELS - META_PIXEL_CHANNELS) as usize {
            return false;
        }
        image.relayout(|picture| picture.meta = meta);
    }
    true
}

/// `CheckMergedImageAlpha`.
fn check_merged_image_alpha(info: &PsdInfo, image: &mut Picture) {
    let grey_alpha = info.mode == GRAYSCALE_MODE && info.channels > 1;
    let colour_alpha = info.mode == RGB_MODE && info.channels > 3;
    let cmyk_alpha = info.mode == CMYK_MODE && info.channels > 4;
    if grey_alpha || colour_alpha || cmyk_alpha {
        image.set_alpha(true);
    }
}

/// `GetPSDPacketSize`.
fn packet_size(image: &Picture) -> usize {
    if image.depth > 16 {
        return 4;
    }
    if image.depth > 8 {
        return 2;
    }
    1
}

/// `GetPSDRowSize`.
fn row_size(image: &Picture) -> usize {
    if image.depth == 1 {
        return image.width.div_ceil(8) * packet_size(image);
    }
    image.width * packet_size(image)
}

/// `ReadPSDChannelPixels`: one row of a channel stored into the image.
fn store_row(image: &mut Picture, row: usize, channel: Channel, pixels: &[u8]) {
    let size = packet_size(image);
    let width = image.width;
    for x in 0..width {
        let at = x * size;
        let value = match size {
            1 => char_quantum(pixels[at]),
            2 => u16::from_be_bytes([pixels[at], pixels[at + 1]]) as f32,
            _ => {
                let float = f32::from_be_bytes([pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]]);
                (QUANTUM_RANGE * float as f64) as f32
            }
        };
        image.set(row * width + x, channel, value);
    }
}

/// `ReadPSDChannelRaw`.
fn read_channel_raw(blob: &mut Blob, image: &mut Picture, channel: Channel) -> bool {
    let size = row_size(image);
    for row in 0..image.height {
        let pixels = blob.read(size);
        if pixels.len() != size {
            return false;
        }
        store_row(image, row, channel, pixels);
    }
    true
}

/// `ReadPSDRLESizes`: the compressed length of each row.
fn read_rle_sizes(blob: &mut Blob, version: u16, count: usize) -> Vec<u64> {
    let mut sizes = Vec::with_capacity(count);
    for _ in 0..count {
        let size = if version == 1 { blob.u16() as u64 } else { blob.u32() as u64 };
        sizes.push(size);
    }
    sizes
}

/// `ReadPSDChannelRLE`.
fn read_channel_rle(blob: &mut Blob, image: &mut Picture, channel: Channel, sizes: &[u64]) -> Result<bool> {
    let size = row_size(image);
    let longest = sizes.iter().take(image.height).copied().max().unwrap_or(0);
    if longest > size as u64 + 2048 {
        bail!("InvalidLength");
    }
    let mut pixels = vec![0u8; size];
    for (row, &row_length) in sizes.iter().enumerate().take(image.height) {
        let compact = blob.read(row_length as usize);
        if compact.len() as u64 != row_length {
            return Ok(false);
        }
        let decoded = decode_packbits(compact, &mut pixels);
        if decoded != size {
            return Ok(false);
        }
        store_row(image, row, channel, &pixels);
    }
    Ok(true)
}

/// `DecodePSDPixels` for whole bytes (the reader never meets a 1-bit
/// image's rows here): PackBits, stopping when a single byte is left.
fn decode_packbits(compact: &[u8], pixels: &mut [u8]) -> usize {
    let number_pixels = pixels.len();
    let mut packets = compact.len() as isize;
    let mut source = 0usize;
    let mut written = 0usize;
    while packets > 1 && written < number_pixels {
        packets -= 1;
        let mut length = compact[source] as usize;
        source += 1;
        if length == 128 {
            continue;
        }
        if length > 128 {
            length = 256 - length + 1;
            if packets == 0 {
                return written;
            }
            packets -= 1;
            let pixel = compact[source];
            source += 1;
            for _ in 0..length {
                if written + 1 > number_pixels {
                    return written;
                }
                pixels[written] = pixel;
                written += 1;
            }
            continue;
        }
        length += 1;
        for _ in 0..length {
            if packets == 0 {
                return written;
            }
            packets -= 1;
            if written + 1 > number_pixels {
                return written;
            }
            pixels[written] = compact[source];
            written += 1;
            source += 1;
        }
    }
    written
}

/// `ReadPSDChannelZip`.
fn read_channel_zip(blob: &mut Blob, image: &mut Picture, channel: Channel, compression: u16, compact_size: u64) -> Result<bool> {
    if compact_size > blob.size() {
        bail!("UnexpectedEndOfFile");
    }
    let size = packet_size(image);
    let row = image.width * size;
    let count = image.height * row;
    let mut pixels = vec![0u8; count];
    let compact = blob.read(compact_size as usize);
    if compact.len() as u64 != compact_size {
        bail!("UnexpectedEndOfFile");
    }
    let mut inflater = flate2::Decompress::new(true);
    loop {
        let consumed = inflater.total_in() as usize;
        let produced = inflater.total_out() as usize;
        if produced >= count {
            break;
        }
        let status = inflater.decompress(&compact[consumed.min(compact.len())..], &mut pixels[produced..], flate2::FlushDecompress::Sync);
        match status {
            Ok(flate2::Status::StreamEnd) => break,
            Ok(flate2::Status::Ok) => {}
            Ok(flate2::Status::BufError) | Err(_) => return Ok(false),
        }
    }
    if compression == ZIP_WITH_PREDICTION {
        match size {
            1 => unpredict_8bit(&mut pixels, image.width, row),
            2 => unpredict_16bit(&mut pixels, image.width, row),
            4 => pixels = unpredict_32bit(&pixels, image.width, image.height, row),
            _ => {}
        }
    }
    for (index, line) in pixels.chunks_exact(row.max(1)).enumerate().take(image.height) {
        store_row(image, index, channel, line);
    }
    Ok(true)
}

/// `Unpredict8Bit`: each byte adds the one before it, row by row.
fn unpredict_8bit(pixels: &mut [u8], columns: usize, row: usize) {
    for line in pixels.chunks_exact_mut(row.max(1)) {
        for x in 1..columns {
            line[x] = line[x].wrapping_add(line[x - 1]);
        }
    }
}

/// `Unpredict16Bit`: big-endian sums with the low byte's carry.
fn unpredict_16bit(pixels: &mut [u8], columns: usize, row: usize) {
    for line in pixels.chunks_exact_mut(row.max(1)) {
        for x in 0..columns.saturating_sub(1) {
            let at = x * 2;
            let carry = ((line[at + 1] as u32 + line[at + 3] as u32) >> 8) as u8;
            line[at + 2] = line[at + 2].wrapping_add(line[at]).wrapping_add(carry);
            line[at + 3] = line[at + 3].wrapping_add(line[at + 1]);
        }
    }
}

/// `Unpredict32Bit`: byte sums across the row, then the four byte planes
/// interleaved back into floats.
fn unpredict_32bit(pixels: &[u8], columns: usize, rows: usize, row: usize) -> Vec<u8> {
    let mut summed = pixels.to_vec();
    let mut output = vec![0u8; pixels.len()];
    for y in 0..rows {
        let start = y * row;
        for x in 1..row {
            summed[start + x] = summed[start + x].wrapping_add(summed[start + x - 1]);
        }
        for x in 0..columns {
            let at = start + x;
            let out = start + x * 4;
            output[out] = summed[at];
            output[out + 1] = summed[at + columns];
            output[out + 2] = summed[at + 2 * columns];
            output[out + 3] = summed[at + 3 * columns];
        }
    }
    output
}

/// `NegateCMYK`: every channel but alpha negated.
fn negate_cmyk(image: &mut Picture) {
    let layout = image.layout();
    let stride = layout.len();
    for pixel in image.data.chunks_exact_mut(stride) {
        for (value, channel) in pixel.iter_mut().zip(layout.iter()) {
            if matches!(channel, Channel::Alpha | Channel::Index) {
                continue;
            }
            *value = (QUANTUM_RANGE - *value as f64) as f32;
        }
    }
}

/// `CorrectPSDAlphaBlend`: semi-transparent sRGB pixels taken back from
/// Photoshop's blend with white.
fn correct_alpha_blend(image: &mut Picture) {
    if !image.alpha || image.space != Space::SRgb {
        return;
    }
    let layout = image.layout();
    let stride = layout.len();
    let Some(alpha_offset) = image.offset(Channel::Alpha) else {
        return;
    };
    for pixel in image.data.chunks_exact_mut(stride) {
        let gamma = QUANTUM_SCALE * pixel[alpha_offset] as f64;
        if gamma == 0.0 || gamma == 1.0 {
            continue;
        }
        for (value, channel) in pixel.iter_mut().zip(layout.iter()) {
            if *channel == Channel::Alpha {
                continue;
            }
            *value = ((*value as f64 - ((1.0 - gamma) * QUANTUM_RANGE)) / gamma) as f32;
        }
    }
}

/// `ReadPSDMergedImage`: whether the merged image was read.
fn read_merged_image(blob: &mut Blob, image: &mut Picture, info: &PsdInfo) -> Result<bool> {
    let compression = blob.u16();
    if compression != RAW && compression != RLE {
        return Ok(false);
    }
    let sizes = if compression == RLE { read_rle_sizes(blob, info.version, image.height * info.channels as usize) } else { Vec::new() };
    if !set_meta_channels(image, info, info.channels) {
        return Ok(false);
    }
    for index in 0..info.channels as usize {
        let Some(channel) = channel_from_index(info.mode, index as i64) else {
            bail!("MaximumChannelsExceeded");
        };
        let read = if compression == RLE {
            let rows = image.height;
            read_channel_rle(blob, image, channel, &sizes[index * rows..(index + 1) * rows])?
        } else {
            read_channel_raw(blob, image, channel)
        };
        if !read {
            return Ok(false);
        }
    }
    if image.space == Space::Cmyk {
        negate_cmyk(image);
    }
    correct_alpha_blend(image);
    Ok(true)
}

/// `GetLayerInfoSize`: the layer information's length, after any
/// transparency (`Mt16`, `Mt32`, `Mtrn`) or 16- and 32-bit layer blocks
/// (`Lr16`, `Lr32`) that lead to it.
fn layer_info_size(blob: &mut Blob, info: &PsdInfo, image: &mut Picture) -> u64 {
    let size = blob.psd_size(info.version);
    if size != 0 {
        return size;
    }
    blob.u32();
    let mut kind = blob.read(4);
    if kind != b"8BIM" {
        return 0;
    }
    kind = blob.read(4);
    if kind == b"Mt16" || kind == b"Mt32" || kind == b"Mtrn" {
        let size = blob.psd_size(info.version);
        if size != 0 {
            return 0;
        }
        image.set_alpha(true);
        kind = blob.read(4);
        if kind != b"8BIM" {
            return 0;
        }
        kind = blob.read(4);
    }
    if kind == b"Lr16" || kind == b"Lr32" {
        return blob.psd_size(info.version);
    }
    0
}

/// `CheckPSDChannels`: the layer has every channel its mode needs.
fn layer_channels_fit(blob: &Blob, info: &PsdInfo, layer: &LayerInfo) -> bool {
    if layer.width == 0 || layer.height == 0 {
        return true;
    }
    if (layer.channels.len() as u16) < info.min_channels {
        return false;
    }
    let mut red = true;
    let mut green = info.min_channels >= 3;
    let mut blue = info.min_channels >= 3;
    let mut black = info.min_channels >= 4;
    let mut index = info.mode == INDEXED_MODE;
    let mut alpha = false;
    for (position, (channel, size)) in layer.channels.iter().enumerate() {
        if *size >= blob.size() {
            return false;
        }
        let Some(channel) = channel else {
            continue;
        };
        if position == 0 && info.mode == INDEXED_MODE {
            if *channel != Channel::Red {
                return false;
            }
            index = false;
        }
        match channel {
            Channel::Alpha => alpha = true,
            Channel::Red => red = false,
            Channel::Green => green = false,
            Channel::Blue => blue = false,
            Channel::Black => black = false,
            _ => {}
        }
    }
    let missing = red || green || blue || black || index;
    if !missing && !alpha {
        return true;
    }
    !missing && alpha && layer.channels.len() as u16 > info.min_channels
}

/// `ReadPSDLayersInternal`: the layers read into `layers` (each with its
/// place and blend mode), unless `skip_layers`, when only whether the
/// merged image has alpha is taken.
fn read_layers(blob: &mut Blob, image: &mut Picture, info: &PsdInfo, skip_layers: bool, layers: &mut Vec<Picture>) -> Result<()> {
    let size = layer_info_size(blob, info, image);
    if size == 0 {
        check_merged_image_alpha(info, image);
        return Ok(());
    }
    let mut count = blob.i16() as i64;
    if count < 0 {
        // The first alpha channel of the merged image is its transparency.
        count = -count;
        image.set_alpha(true);
    }
    if skip_layers {
        return Ok(());
    }
    if count == 0 {
        bail!("InvalidNumberOfLayers");
    }
    let mut records = Vec::with_capacity(count as usize);
    for _ in 0..count {
        records.push(read_layer_record(blob, info)?);
    }
    // Each layer that is not empty is a clone of the image at its size.
    let mut pictures: Vec<Option<Picture>> = Vec::with_capacity(records.len());
    for record in &records {
        if record.width == 0 || record.height == 0 {
            pictures.push(None);
            continue;
        }
        let mut picture = Picture { width: record.width as usize, height: record.height as usize, data: Vec::new(), ..image.clone() };
        picture.data = vec![0.0; picture.width * picture.height * picture.stride()];
        if record.channels.iter().any(|(channel, _)| *channel == Some(Channel::Alpha)) {
            picture.set_alpha(true);
        }
        pictures.push(Some(picture));
    }
    for (record, picture) in records.iter().zip(pictures.iter_mut()) {
        let Some(picture) = picture.as_mut() else {
            for (_, size) in &record.channels {
                if !blob.discard(*size) {
                    bail!("UnexpectedEndOfFile");
                }
            }
            continue;
        };
        if !read_layer(blob, info, record, picture)? {
            bail!("UnableToReadLayer");
        }
    }
    // `AttachPSDLayers`: the layers that are not empty, at their places.
    for (record, picture) in records.iter().zip(pictures) {
        if let Some(mut picture) = picture {
            picture.page = (record.left, record.top);
            layers.push(picture);
        }
    }
    Ok(())
}

/// A layer record: its rectangle, channels, blend mode, opacity,
/// visibility and mask.
fn read_layer_record(blob: &mut Blob, info: &PsdInfo) -> Result<LayerInfo> {
    let top = blob.i32() as i64;
    let left = blob.i32() as i64;
    let bottom = blob.i32() as i64;
    let right = blob.i32() as i64;
    if right < left || bottom < top {
        bail!("ImproperImageHeader");
    }
    let mut layer = LayerInfo { left, top, width: (right - left) as u64, height: (bottom - top) as u64, ..LayerInfo::default() };
    let channels = blob.u16();
    if channels > MAX_PSD_CHANNELS {
        bail!("MaximumChannelsExceeded");
    }
    for _ in 0..channels {
        let id = blob.i16() as i64;
        let channel = channel_from_index(info.mode, id);
        let size = blob.psd_size(info.version);
        layer.channels.push((channel, size));
    }
    if !layer_channels_fit(blob, info, &layer) {
        bail!("ImproperImageHeader");
    }
    let kind = blob.read(4);
    if kind != b"8BIM" {
        bail!("ImproperImageHeader");
    }
    let key = blob.read(4);
    if key.len() != 4 {
        bail!("ImproperImageHeader");
    }
    layer.blend_key = key.to_vec();
    layer.opacity = char_quantum(blob.byte_or_eof());
    blob.byte_or_eof();
    let flags = blob.byte_or_eof();
    layer.visible = flags & 0x02 == 0;
    blob.byte();
    let size = blob.u32() as u64;
    if size == 0 {
        return Ok(layer);
    }
    let length = blob.u32() as u64;
    let mut combined = length + 4;
    if length != 0 {
        let mask_top = blob.i32() as i64;
        let mask_left = blob.i32() as i64;
        let mask_bottom = blob.i32() as i64;
        let mask_right = blob.i32() as i64;
        layer.mask.top = mask_top;
        layer.mask.left = mask_left;
        layer.mask.height = (mask_bottom - mask_top) as u64;
        layer.mask.width = (mask_right - mask_left) as u64;
        layer.mask.background = blob.byte_or_eof();
        layer.mask.flags = blob.byte_or_eof();
        if layer.mask.flags & 0x01 == 0 {
            layer.mask.top -= layer.top;
            layer.mask.left -= layer.left;
        }
        if !blob.discard(length.wrapping_sub(18)) {
            bail!("UnexpectedEndOfFile");
        }
    }
    let length = blob.u32() as u64;
    combined += length + 4;
    if length != 0 && !blob.discard(length) {
        bail!("UnexpectedEndOfFile");
    }
    // The name: a Pascal string padded to four bytes (padding skipped only
    // for a name that is not empty).
    let mut length = blob.byte_or_eof() as u64;
    combined += length + 1;
    if length > 0 {
        blob.read(length as usize);
        length += 1;
    }
    if !length.is_multiple_of(4) {
        let padding = 4 - length % 4;
        combined += padding;
        if !blob.discard(padding) {
            bail!("UnexpectedEndOfFile");
        }
    }
    let rest = size.wrapping_sub(combined);
    if rest > 0 {
        if rest > blob.size() {
            bail!("InsufficientImageDataInFile");
        }
        blob.read(rest as usize);
    }
    Ok(layer)
}

/// `ReadPSDLayer`: the layer's channels, opacity and mask; false when IM
/// stops reading (a colormapped layer).
fn read_layer(blob: &mut Blob, info: &PsdInfo, record: &LayerInfo, picture: &mut Picture) -> Result<bool> {
    if info.mode != INDEXED_MODE {
        picture.fill_background(false);
    }
    picture.compose = if record.visible { Compose::from_blend_key(&record.blend_key) } else { Compose::No };
    if !set_meta_channels(picture, info, record.channels.len() as u16) {
        return Ok(false);
    }
    let mut mask: Option<Picture> = None;
    for (channel, size) in &record.channels {
        let compression = blob.u16();
        if let Some(read_mask) = read_channel(blob, info, record, picture, *channel, *size, compression)? {
            mask = Some(read_mask);
        }
    }
    apply_layer_opacity(picture, record.opacity);
    if picture.space == Space::Cmyk {
        negate_cmyk(picture);
    }
    if let Some(mask) = mask
        && record.mask.flags & 0x02 == 0
    {
        let background = if record.mask.background == 0 { 0.0 } else { QUANTUM_RANGE };
        apply_opacity_mask(picture, &mask, (record.mask.left, record.mask.top), background);
    }
    Ok(true)
}

/// `ReadPSDChannel` for a layer: the channel's data read into the layer,
/// or into a new mask image (returned) for its user mask.
fn read_channel(blob: &mut Blob, info: &PsdInfo, record: &LayerInfo, picture: &mut Picture, channel: Option<Channel>, size: u64, compression: u16) -> Result<Option<Picture>> {
    let end_offset = size as i64 - 2;
    let Some(channel) = channel else {
        blob.skip_signed(end_offset);
        return Ok(None);
    };
    let mut mask: Option<Picture> = None;
    let mut target_channel = channel;
    if channel == Channel::ReadMask {
        let unusable = record.mask.flags > 2 || record.mask.flags & 0x02 != 0 || record.mask.width < 1 || record.mask.height < 1;
        if unusable {
            blob.skip_signed(end_offset);
            return Ok(None);
        }
        // A grey image of the mask's size (`SetImageType(GrayscaleType)`);
        // when IM cannot make one, the data goes to the layer's read mask,
        // which it does not have.
        let fits = record.mask.width <= MASK_LIMIT && record.mask.height <= MASK_LIMIT;
        if fits {
            let mut grey = Picture { width: record.mask.width as usize, height: record.mask.height as usize, data: Vec::new(), ..picture.clone() };
            grey.alpha = false;
            grey.pseudo = false;
            grey.meta = 0;
            grey.space = Space::Gray;
            grey.data = vec![0.0; grey.width * grey.height];
            mask = Some(grey);
            target_channel = Channel::Red;
        }
    }
    let offset = blob.tell();
    let target = mask.as_mut().unwrap_or(picture);
    let read = match compression {
        RAW => read_channel_raw(blob, target, target_channel),
        RLE => {
            let sizes = read_rle_sizes(blob, info.version, target.height);
            read_channel_rle(blob, target, target_channel, &sizes)?
        }
        ZIP_WITHOUT_PREDICTION | ZIP_WITH_PREDICTION => {
            let compact_size = u64::try_from(end_offset).unwrap_or(u64::MAX);
            read_channel_zip(blob, target, target_channel, compression, compact_size)?
        }
        _ => false,
    };
    let end = offset as i64 + end_offset;
    if end >= 0 {
        blob.seek(end as u64);
    }
    if !read {
        bail!("UnableToDecompressImage");
    }
    Ok(mask)
}

/// `ApplyPSDLayerOpacity`: alpha scaled by the layer's opacity (an image
/// without alpha gets it, opaque, first).
fn apply_layer_opacity(picture: &mut Picture, opacity: f32) {
    if opacity == QUANTUM_RANGE as f32 {
        return;
    }
    if !picture.alpha {
        picture.set_alpha(true);
        let offset = picture.offset(Channel::Alpha).unwrap_or(0);
        let stride = picture.stride();
        for pixel in picture.data.chunks_exact_mut(stride) {
            pixel[offset] = QUANTUM_RANGE as f32;
        }
    }
    let offset = picture.offset(Channel::Alpha).unwrap_or(0);
    let stride = picture.stride();
    for pixel in picture.data.chunks_exact_mut(stride) {
        pixel[offset] = (QUANTUM_SCALE * pixel[offset] as f64 * opacity as f64) as f32;
    }
}

/// `ApplyPSDOpacityMask`: alpha scaled by the mask's intensity, the mask
/// laid over a copy of the layer filled with its background (red only).
fn apply_opacity_mask(picture: &mut Picture, mask: &Picture, place: (i64, i64), background: f64) {
    if !picture.alpha {
        return;
    }
    // `SetImageColor` with a colour whose red is the background: a grey
    // image ends with the blue (zero) in its one colour channel.
    let mut complete = picture.clone();
    let layout = complete.layout();
    let stride = layout.len();
    let red = if complete.space == Space::Gray { 0.0 } else { background as f32 };
    for pixel in complete.data.chunks_exact_mut(stride) {
        for (value, channel) in pixel.iter_mut().zip(layout.iter()) {
            match channel {
                Channel::Red => *value = red,
                Channel::Green | Channel::Blue | Channel::Black => *value = 0.0,
                Channel::Alpha => *value = QUANTUM_RANGE as f32,
                _ => {}
            }
        }
    }
    composite(&mut complete, mask, Compose::Over, place.0, place.1);
    let alpha_offset = picture.offset(Channel::Alpha).unwrap_or(0);
    let picture_stride = picture.stride();
    for index in 0..picture.width * picture.height {
        let intensity = pixel_intensity(&complete, index);
        let alpha = picture.data[index * picture_stride + alpha_offset] as f64;
        picture.data[index * picture_stride + alpha_offset] = (intensity * (QUANTUM_SCALE * alpha)) as f32;
    }
}

/// `GetPixelIntensity` (Rec. 709 luma, the default).
fn pixel_intensity(picture: &Picture, index: usize) -> f64 {
    let stride = picture.stride();
    let pixel = &picture.data[index * stride..(index + 1) * stride];
    let red = pixel[picture.offset(Channel::Red).unwrap_or(0)] as f64;
    if stride == 1 {
        return red;
    }
    let green = pixel[picture.offset(Channel::Green).unwrap_or(0)] as f64;
    let blue = pixel[picture.offset(Channel::Blue).unwrap_or(0)] as f64;
    0.212656 * red + 0.715158 * green + 0.072186 * blue
}

/// `MergeImageLayers(FlattenLayer)`: a transparent canvas the image's
/// size, with the image and each layer composed on it in turn.
fn flatten(image: Picture, layers: Vec<Picture>) -> Picture {
    let mut canvas = image.clone();
    canvas.fill_background(true);
    canvas.page = (0, 0);
    composite(&mut canvas, &image, image.compose, image.page.0, image.page.1);
    for layer in &layers {
        composite(&mut canvas, layer, layer.compose, layer.page.0, layer.page.1);
    }
    canvas
}

/// `ClampPixel`.
fn clamp_pixel(value: f64) -> f32 {
    if value < 0.0 {
        return 0.0;
    }
    if value >= QUANTUM_RANGE {
        return QUANTUM_RANGE as f32;
    }
    value as f32
}

/// `MagickSafeReciprocal`.
fn safe_reciprocal(value: f64) -> f64 {
    if value > -MAGICK_EPSILON && value < MAGICK_EPSILON {
        return 1.0 / MAGICK_EPSILON;
    }
    1.0 / value
}

/// `RoundToUnity`.
fn round_to_unity(value: f64) -> f64 {
    if value < 0.0 {
        return 0.0;
    }
    if value > 1.0 {
        return 1.0;
    }
    value
}

/// `CompositeImage(image, source, compose, clip_to_self, x, y)`: the
/// source (relabelled to the canvas's colorspace) composed onto the canvas
/// where it lies.
fn composite(canvas: &mut Picture, source: &Picture, compose: Compose, x_offset: i64, y_offset: i64) {
    let mut source = source.clone();
    if source.space != canvas.space {
        source.set_space(canvas.space);
    }
    let canvas_layout = canvas.layout();
    let canvas_stride = canvas_layout.len();
    let source_stride = source.stride();
    // Where each canvas channel is in the source, and the alphas.
    let offsets = Offsets {
        source: canvas_layout.iter().map(|channel| source.offset(*channel)).collect(),
        source_alpha: source.offset(Channel::Alpha),
        canvas_alpha: canvas.offset(Channel::Alpha),
        canvas_rgb: rgb_offsets(canvas),
        source_rgb: rgb_offsets(&source),
    };
    for y in 0..canvas.height as i64 {
        if y < y_offset || y - y_offset >= source.height as i64 {
            continue;
        }
        let source_row = (y - y_offset) as usize;
        for x in 0..canvas.width as i64 {
            if x < x_offset {
                continue;
            }
            if x - x_offset >= source.width as i64 {
                break;
            }
            let source_index = source_row * source.width + (x - x_offset) as usize;
            let canvas_index = y as usize * canvas.width + x as usize;
            let source_pixel = &source.data[source_index * source_stride..(source_index + 1) * source_stride];
            let pixel = &mut canvas.data[canvas_index * canvas_stride..(canvas_index + 1) * canvas_stride];
            if compose == Compose::Over {
                compose_over(pixel, &canvas_layout, &offsets, source_pixel);
            } else {
                compose_general(pixel, &canvas_layout, &offsets, source_pixel, compose);
            }
        }
    }
}

/// Where a composite finds what it needs in each pixel.
struct Offsets {
    /// Each canvas channel's offset in the source, none when the source
    /// lacks it (`GetPixelChannelTraits` undefined).
    source: Vec<Option<usize>>,
    source_alpha: Option<usize>,
    canvas_alpha: Option<usize>,
    /// Red, green and blue (`GetPixelInfoPixel`; a grey image's are its
    /// grey).
    canvas_rgb: [usize; 3],
    source_rgb: [usize; 3],
}

fn rgb_offsets(picture: &Picture) -> [usize; 3] {
    let red = picture.offset(Channel::Red).unwrap_or(0);
    let green = picture.offset(Channel::Green).unwrap_or(0);
    let blue = picture.offset(Channel::Blue).unwrap_or(0);
    [red, green, blue]
}

/// `GetPixelAlpha`: opaque without alpha.
fn alpha_of(pixel: &[f32], offset: Option<usize>) -> f64 {
    offset.map_or(QUANTUM_RANGE, |offset| pixel[offset] as f64)
}

/// `CompositeOverImage` for one pixel.
fn compose_over(pixel: &mut [f32], layout: &[Channel], offsets: &Offsets, source_pixel: &[f32]) {
    let sa = QUANTUM_SCALE * alpha_of(source_pixel, offsets.source_alpha);
    let da = QUANTUM_SCALE * alpha_of(pixel, offsets.canvas_alpha);
    let alpha = sa + da - sa * da;
    for (index, channel) in layout.iter().enumerate() {
        if *channel == Channel::Alpha {
            pixel[index] = clamp_pixel(QUANTUM_RANGE * alpha);
            continue;
        }
        let Some(source_offset) = offsets.source[index] else {
            continue;
        };
        let sc = source_pixel[source_offset] as f64;
        let value = &mut pixel[index];
        if *channel == Channel::Index {
            *value = sc as f32;
            continue;
        }
        let dc = *value as f64;
        let sca = QUANTUM_SCALE * sa * sc;
        let dca = QUANTUM_SCALE * da * dc;
        let gamma = safe_reciprocal(alpha);
        *value = clamp_pixel(QUANTUM_RANGE * gamma * (sca + dca * (1.0 - sa)));
    }
}

/// `ConvertRGBToHCL`.
fn rgb_to_hcl(red: f64, green: f64, blue: f64) -> (f64, f64, f64) {
    let max = red.max(green.max(blue));
    let c = max - red.min(green.min(blue));
    let mut h = 0.0;
    if c.abs() < MAGICK_EPSILON {
        h = 0.0;
    } else if (red - max).abs() < MAGICK_EPSILON {
        h = ((green - blue) / c + 6.0) % 6.0;
    } else if (green - max).abs() < MAGICK_EPSILON {
        h = ((blue - red) / c) + 2.0;
    } else if (blue - max).abs() < MAGICK_EPSILON {
        h = ((red - green) / c) + 4.0;
    }
    let hue = h / 6.0;
    let chroma = QUANTUM_SCALE * c;
    let luma = QUANTUM_SCALE * (0.298839 * red + 0.586811 * green + 0.114350 * blue);
    (hue, chroma, luma)
}

/// `ConvertHCLToRGB`.
fn hcl_to_rgb(hue: f64, chroma: f64, luma: f64) -> (f64, f64, f64) {
    let h = 6.0 * hue;
    let c = chroma;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());
    let (mut r, mut g, mut b) = (0.0, 0.0, 0.0);
    if (0.0..1.0).contains(&h) {
        r = c;
        g = x;
    } else if (1.0..2.0).contains(&h) {
        r = x;
        g = c;
    } else if (2.0..3.0).contains(&h) {
        g = c;
        b = x;
    } else if (3.0..4.0).contains(&h) {
        g = x;
        b = c;
    } else if (4.0..5.0).contains(&h) {
        r = x;
        b = c;
    } else if (5.0..6.0).contains(&h) {
        r = c;
        b = x;
    }
    let m = luma - (0.298839 * r + 0.586811 * g + 0.114350 * b);
    (QUANTUM_RANGE * (r + m), QUANTUM_RANGE * (g + m), QUANTUM_RANGE * (b + m))
}

/// A pixel's red, green and blue.
fn pixel_rgb(pixel: &[f32], offsets: [usize; 3]) -> (f64, f64, f64) {
    (pixel[offsets[0]] as f64, pixel[offsets[1]] as f64, pixel[offsets[2]] as f64)
}

/// `CompositeImage`'s general loop for one pixel (`compose:sync` and
/// clamping on, as by default).
fn compose_general(pixel: &mut [f32], layout: &[Channel], offsets: &Offsets, source_pixel: &[f32], compose: Compose) {
    let sa = QUANTUM_SCALE * alpha_of(source_pixel, offsets.source_alpha);
    let da = QUANTUM_SCALE * alpha_of(pixel, offsets.canvas_alpha);
    let alpha = match compose {
        Compose::Dissolve => sa * (-da) + sa + da,
        Compose::Hue | Compose::Saturate | Compose::Colorize | Compose::Luminize | Compose::No | Compose::Over => 1.0,
        _ => round_to_unity(sa + da - sa * da),
    };
    // The colour operators work from both pixels' red, green and blue.
    let colour_operator = matches!(compose, Compose::Hue | Compose::Saturate | Compose::Colorize | Compose::Luminize);
    let mut colour = (0.0, 0.0, 0.0);
    if colour_operator {
        let canvas_rgb = pixel_rgb(pixel, offsets.canvas_rgb);
        let source_rgb = pixel_rgb(source_pixel, offsets.source_rgb);
        let (canvas_hue, canvas_chroma, canvas_luma) = rgb_to_hcl(canvas_rgb.0, canvas_rgb.1, canvas_rgb.2);
        let (source_hue, source_chroma, source_luma) = rgb_to_hcl(source_rgb.0, source_rgb.1, source_rgb.2);
        colour = match compose {
            Compose::Colorize => hcl_to_rgb(source_hue, source_chroma, canvas_luma),
            Compose::Hue => hcl_to_rgb(source_hue, canvas_chroma, canvas_luma),
            Compose::Luminize => hcl_to_rgb(canvas_hue, canvas_chroma, source_luma),
            _ => hcl_to_rgb(canvas_hue, source_chroma, canvas_luma),
        };
    }
    let transparent_source = (QUANTUM_RANGE * sa).abs() < MAGICK_EPSILON;
    let transparent_canvas = (QUANTUM_RANGE * da).abs() < MAGICK_EPSILON;
    for (index, channel) in layout.iter().enumerate() {
        if *channel == Channel::Alpha {
            let result = match compose {
                Compose::No => QUANTUM_RANGE * da,
                Compose::Hue | Compose::Saturate | Compose::Colorize | Compose::Luminize => {
                    if transparent_source {
                        QUANTUM_RANGE * da
                    } else if transparent_canvas || sa >= da {
                        QUANTUM_RANGE * sa
                    } else {
                        QUANTUM_RANGE * da
                    }
                }
                Compose::Difference => QUANTUM_RANGE * (sa - da).abs(),
                _ => QUANTUM_RANGE * alpha,
            };
            pixel[index] = clamp_pixel(result);
            continue;
        }
        let Some(source_offset) = offsets.source[index] else {
            continue;
        };
        let sc = source_pixel[source_offset] as f64;
        let value = &mut pixel[index];
        let dc = *value as f64;
        if *channel == Channel::Index {
            continue;
        }
        let sca = QUANTUM_SCALE * sa * sc;
        let dca = QUANTUM_SCALE * da * dc;
        let gamma = match compose {
            Compose::Darken | Compose::Lighten => safe_reciprocal(1.0 - alpha),
            _ => safe_reciprocal(alpha),
        };
        let result = blend(compose, *channel, sc, dc, sa, da, sca, dca, gamma, colour, transparent_source, transparent_canvas);
        *value = clamp_pixel(result);
    }
}

/// One colour channel of the general composite (the operators PSD blend
/// modes become).
#[allow(clippy::too_many_arguments)]
fn blend(compose: Compose, channel: Channel, sc: f64, dc: f64, sa: f64, da: f64, sca: f64, dca: f64, gamma: f64, colour: (f64, f64, f64), transparent_source: bool, transparent_canvas: bool) -> f64 {
    let unpremultiplied = |value: f64, alpha: f64| if alpha > 0.0 { value / alpha } else { 0.0 };
    match compose {
        Compose::ColorBurn => {
            let d = if da > 0.0 { round_to_unity(dca / da) } else { 0.0 };
            let s = if sa > 0.0 { round_to_unity(sca / sa) } else { 0.0 };
            let blend = if s <= 0.0 { 0.0 } else { 1.0 - (1.0 - d) / s };
            QUANTUM_RANGE * gamma * round_to_unity(sa * da * round_to_unity(blend) + sa * (1.0 - da) * s + da * (1.0 - sa) * d)
        }
        Compose::ColorDodge => {
            let s = if sa > 0.0 { round_to_unity(sca / sa) } else { 0.0 };
            let d = if da > 0.0 { round_to_unity(dca / da) } else { 0.0 };
            let blend = if s >= 1.0 {
                1.0
            } else if d <= 0.0 {
                0.0
            } else if (1.0 - s) <= 0.0 {
                1.0
            } else {
                let ratio = d / (1.0 - s);
                if 1.0 < ratio { 1.0 } else { ratio }
            };
            QUANTUM_RANGE * gamma * (sa * da * blend + sca * (1.0 - da) + dca * (1.0 - sa))
        }
        Compose::Colorize | Compose::Hue | Compose::Luminize | Compose::Saturate => {
            if transparent_source {
                return dc;
            }
            if transparent_canvas {
                return sc;
            }
            match channel {
                Channel::Red => colour.0,
                Channel::Green => colour.1,
                Channel::Blue => colour.2,
                _ => dc,
            }
        }
        Compose::Darken => {
            let smaller = if sca < dca { sca } else { dca };
            QUANTUM_RANGE * round_to_unity(smaller + sca * (1.0 - da) + dca * (1.0 - sa))
        }
        Compose::Difference => {
            let s = unpremultiplied(sca, sa);
            let d = unpremultiplied(dca, da);
            QUANTUM_RANGE * round_to_unity((s - d).abs())
        }
        Compose::Dissolve => gamma * (sa * sc - sa * da * dc + da * dc),
        Compose::Exclusion => {
            let s = unpremultiplied(sca, sa);
            let d = unpremultiplied(dca, da);
            let blend = round_to_unity(s + d - 2.0 * s * d);
            QUANTUM_RANGE * round_to_unity((blend * sa + d * (1.0 - sa)) * (sa + da - sa * da))
        }
        Compose::HardLight => {
            let d = unpremultiplied(dca, da);
            let s = unpremultiplied(sca, sa);
            let blend = if s <= 0.5 { 2.0 * s * d } else { 1.0 - 2.0 * (1.0 - s) * (1.0 - d) };
            QUANTUM_RANGE * gamma * round_to_unity((1.0 - da) * sca + (1.0 - sa) * dca + blend * sa * da)
        }
        Compose::HardMix => {
            let blend = if sca < 0.5 {
                if (2.0 * sca) == 0.0 { 0.0 } else { 1.0 - (1.0 - dca) / (2.0 * sca) }
            } else if (1.0 - ((2.0 * sca) - 1.0)) == 0.0 {
                1.0
            } else {
                dca / (1.0 - (2.0 * sca - 1.0))
            };
            gamma * if blend < 0.5 { 0.0 } else { QUANTUM_RANGE }
        }
        Compose::LinearBurn => QUANTUM_RANGE * (sca + dca - sa * da),
        Compose::LinearDodge => gamma * (sa * sc + da * dc),
        Compose::LinearLight => {
            let d = unpremultiplied(dca, da);
            let s = unpremultiplied(sca, sa);
            QUANTUM_RANGE * gamma * (round_to_unity(d + 2.0 * s - 1.0) * da)
        }
        Compose::Lighten => {
            let s = unpremultiplied(sca, sa);
            let d = unpremultiplied(dca, da);
            let larger = if s > d { s } else { d };
            QUANTUM_RANGE * round_to_unity(sca * (1.0 - da) + dca * (1.0 - sa) + larger * sa * da)
        }
        Compose::Multiply => QUANTUM_RANGE * (sca * dca + sca * (1.0 - da) + dca * (1.0 - sa)),
        Compose::No => QUANTUM_RANGE * dca,
        Compose::Overlay => {
            if (2.0 * dca) < da {
                return QUANTUM_RANGE * gamma * (2.0 * dca * sca + dca * (1.0 - sa) + sca * (1.0 - da));
            }
            QUANTUM_RANGE * gamma * (da * sa - 2.0 * (sa - sca) * (da - dca) + dca * (1.0 - sa) + sca * (1.0 - da))
        }
        Compose::PinLight => {
            let d = if da > 0.0 { round_to_unity(dca / da) } else { 0.0 };
            let s = if sa > 0.0 { round_to_unity(sca / sa) } else { 0.0 };
            let blend = if s <= 0.5 {
                let twice = 2.0 * s;
                if d < twice { d } else { twice }
            } else {
                let stretched = 2.0 * s - 1.0;
                if d > stretched { d } else { stretched }
            };
            QUANTUM_RANGE * gamma * (blend * round_to_unity(sa + da - sa * da))
        }
        Compose::Screen => {
            let s = unpremultiplied(sca, sa);
            let d = unpremultiplied(dca, da);
            if (sa + da - sa * da) > 0.0 { QUANTUM_RANGE * round_to_unity((sa + da - sa * da) * (s + d - s * d)) } else { 0.0 }
        }
        Compose::SoftLight => {
            let source_unit = round_to_unity(sca);
            let canvas_unit = round_to_unity(dca);
            if source_unit <= 0.5 {
                return QUANTUM_RANGE * gamma * (canvas_unit * sa + da * (canvas_unit - (1.0 - 2.0 * source_unit) * canvas_unit * (1.0 - canvas_unit)) + source_unit * (1.0 - da) + canvas_unit * (1.0 - sa));
            }
            let blend = if canvas_unit > 0.25 { canvas_unit.sqrt() } else { ((16.0 * canvas_unit - 12.0) * canvas_unit + 4.0) * canvas_unit };
            QUANTUM_RANGE * gamma * (canvas_unit * sa + da * (canvas_unit + (2.0 * source_unit - 1.0) * (round_to_unity(blend) - canvas_unit)) + source_unit * (1.0 - da) + canvas_unit * (1.0 - sa))
        }
        Compose::VividLight => {
            if sa.abs() < MAGICK_EPSILON || da.abs() < MAGICK_EPSILON {
                return QUANTUM_RANGE * gamma * (sa * da + sca * (1.0 - da) + dca * (1.0 - sa));
            }
            let s = round_to_unity(sca / sa);
            let d = round_to_unity(dca / da);
            let blend = if s <= 0.0 {
                0.0
            } else if s < 0.5 {
                1.0 - (1.0 - d) / (2.0 * s)
            } else if s < 1.0 {
                d / (2.0 * (1.0 - s))
            } else {
                1.0
            };
            QUANTUM_RANGE * gamma * (sa * da * round_to_unity(blend) + sca * (1.0 - da) + dca * (1.0 - sa))
        }
        Compose::Over => {
            if (sa + da * (1.0 - sa)) <= MAGICK_EPSILON {
                return 0.0;
            }
            QUANTUM_RANGE * gamma * ((sca + dca * (1.0 - sa)) / (sa + da * (1.0 - sa)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packbits_runs_and_literals_decode() {
        // A literal of three, a run of four, a no-op, a literal of one.
        let compact = [2, 10, 20, 30, 0xfd, 7, 128, 0, 99, 0];
        let mut pixels = [0u8; 8];
        let written = decode_packbits(&compact, &mut pixels);
        assert_eq!(written, 8);
        assert_eq!(pixels, [10, 20, 30, 7, 7, 7, 7, 99]);
    }

    #[test]
    fn channels_follow_the_mode() {
        assert_eq!(channel_from_index(RGB_MODE, 3), Some(Channel::Alpha));
        assert_eq!(channel_from_index(RGB_MODE, 4), Some(Channel::Meta(0)));
        assert_eq!(channel_from_index(GRAYSCALE_MODE, 1), Some(Channel::Alpha));
        assert_eq!(channel_from_index(CMYK_MODE, 3), Some(Channel::Black));
        assert_eq!(channel_from_index(RGB_MODE, -2), Some(Channel::ReadMask));
        assert_eq!(channel_from_index(RGB_MODE, -3), None);
    }

    #[test]
    fn a_grey_image_shares_its_colour_offsets() {
        let mut picture = Picture::new(2, 1);
        picture.set_space(Space::Gray);
        picture.set(1, Channel::Green, 5.0);
        assert_eq!(picture.data, vec![0.0, 5.0]);
        picture.set_space(Space::SRgb);
        assert_eq!(picture.data, vec![0.0, 0.0, 0.0, 5.0, 5.0, 5.0]);
    }

    /// Writes what the reader leaves for every `.psd` and `.psb` in
    /// `$NERTHUS_PSD_DIR` as `<file>.ours.raw` with a `.ours.json` beside
    /// it, for comparing with ImageMagick's (the oracle's `pixels`).
    #[test]
    #[ignore]
    fn dump_pixel_caches() {
        let directory = std::env::var("NERTHUS_PSD_DIR").expect("set NERTHUS_PSD_DIR");
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&directory).unwrap().flatten().map(|entry| entry.path()).collect();
        files.retain(|path| path.extension().is_some_and(|extension| extension == "psd" || extension == "psb"));
        files.sort();
        for path in files {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let json_path = path.with_file_name(format!("{name}.ours.json"));
            match read(&path) {
                Ok(cache) => {
                    let bytes: Vec<u8> = cache.data.iter().flat_map(|value| value.to_le_bytes()).collect();
                    std::fs::write(path.with_file_name(format!("{name}.ours.raw")), bytes).unwrap();
                    let channels: Vec<String> = cache.channels.iter().map(|channel| format!("{channel:?}")).collect();
                    let json = format!("{{\"width\":{},\"height\":{},\"channels\":{:?},\"colorspace\":\"{:?}\",\"gamma\":{}}}", cache.width, cache.height, channels, cache.colorspace, cache.gamma);
                    std::fs::write(&json_path, json).unwrap();
                }
                Err(error) => std::fs::write(&json_path, format!("{{\"error\":\"{error}\"}}")).unwrap(),
            }
        }
    }
}
