use super::*;

const EMR_SETMAPMODE: u32 = 17;
const EMR_SETWINDOWEXTEX: u32 = 9;
const EMR_SETVIEWPORTEXTEX: u32 = 11;
const EMR_SELECTOBJECT: u32 = 37;
const EMR_CREATEPEN: u32 = 38;
const EMR_CREATEBRUSHINDIRECT: u32 = 39;
const EMR_ELLIPSE: u32 = 42;
const EMR_RECTANGLE: u32 = 43;
const EMR_STRETCHDIBITS: u32 = 81;
const EMR_EXTCREATEFONTINDIRECTW: u32 = 82;
const EMR_EXTTEXTOUTW: u32 = 84;
/// The null pen and white brush among the stock objects.
const NULL_PEN: u32 = 0x8000_0008;

/// Writes a metafile record by record.
struct Builder {
    header: Vec<u8>,
    records: Vec<Vec<u8>>,
}

fn words(values: &[i32]) -> Vec<u8> {
    values.iter().flat_map(|value| value.to_le_bytes()).collect()
}

impl Builder {
    /// A metafile whose frame is `frame` (hundredths of a millimetre) on
    /// a 96 dpi reference device.
    fn new(frame: [i32; 4]) -> Builder {
        let mut header = Vec::new();
        header.extend(words(&[EMR_HEADER as i32, 108]));
        header.extend(words(&[0, 0, 0, 0]));
        header.extend(words(&frame));
        header.extend(SIGNATURE.to_le_bytes());
        header.extend(words(&[0x10000, 0, 0]));
        header.extend(16u16.to_le_bytes());
        header.extend(0u16.to_le_bytes());
        header.extend(words(&[0, 0, 0]));
        header.extend(words(&[960, 960, 254, 254]));
        header.extend(words(&[0, 0, 0, 254_000, 254_000]));
        Builder { header, records: Vec::new() }
    }

    fn record(&mut self, kind: u32, payload: &[u8]) {
        let mut record = Vec::new();
        let padded = payload.len().next_multiple_of(4);
        record.extend(kind.to_le_bytes());
        record.extend(((8 + padded) as u32).to_le_bytes());
        record.extend(payload);
        record.resize(8 + padded, 0);
        self.records.push(record);
    }

    /// An EMF+ comment holding these EMF+ records (type, flags, data).
    fn plus(&mut self, records: &[(u16, u16, Vec<u8>)]) {
        let mut body = Vec::new();
        for (kind, flags, data) in records {
            body.extend(kind.to_le_bytes());
            body.extend(flags.to_le_bytes());
            body.extend(((12 + data.len()) as u32).to_le_bytes());
            body.extend((data.len() as u32).to_le_bytes());
            body.extend(data);
        }
        let mut payload = ((4 + body.len()) as u32).to_le_bytes().to_vec();
        payload.extend(EMF_PLUS.to_le_bytes());
        payload.extend(body);
        self.record(EMR_COMMENT, &payload);
    }

    fn finish(mut self) -> Vec<u8> {
        self.record(EMR_EOF, &words(&[0, 16, 20]));
        let total: usize = self.header.len() + self.records.iter().map(Vec::len).sum::<usize>();
        self.header[48..52].copy_from_slice(&(total as u32).to_le_bytes());
        self.header[52..56].copy_from_slice(&(self.records.len() as u32 + 1).to_le_bytes());
        let mut bytes = self.header;
        for record in self.records {
            bytes.extend(record);
        }
        bytes
    }
}

fn render(bytes: &[u8]) -> Pixmap {
    let header = Header::parse(bytes).unwrap();
    let (width, height) = header.picture_size();
    let mut pixmap = Pixmap::new(width as u32, height as u32).unwrap();
    pixmap.fill(Color::WHITE);
    play(bytes, &header, &mut pixmap, 0);
    pixmap
}

fn rgb(pixmap: &Pixmap, x: u32, y: u32) -> [u8; 3] {
    let pixel = pixmap.pixel(x, y).unwrap().demultiply();
    [pixel.red(), pixel.green(), pixel.blue()]
}

fn solid_brush(builder: &mut Builder, handle: i32, colour: u32) {
    builder.record(EMR_CREATEBRUSHINDIRECT, &words(&[handle, 0, colour as i32, 0]));
    builder.record(EMR_SELECTOBJECT, &words(&[handle]));
}

#[test]
fn the_picture_is_the_frame_at_the_devices_resolution() {
    // One inch by half an inch at 96 dpi.
    let bytes = Builder::new([0, 0, 2540, 1270]).finish();
    let pixmap = render(&bytes);
    assert_eq!((pixmap.width(), pixmap.height()), (96, 48));
    assert_eq!(rgb(&pixmap, 50, 20), [255, 255, 255]);
}

#[test]
fn rectangles_leave_out_their_right_and_bottom_edges() {
    let mut builder = Builder::new([0, 0, 2540, 2540]);
    solid_brush(&mut builder, 1, 0x0000_00FF);
    builder.record(EMR_SELECTOBJECT, &words(&[NULL_PEN as i32]));
    builder.record(EMR_RECTANGLE, &words(&[10, 10, 20, 20]));
    // A one-pixel black pen draws the edges it keeps.
    builder.record(EMR_CREATEPEN, &words(&[2, 0, 0, 0, 0]));
    builder.record(EMR_SELECTOBJECT, &words(&[2]));
    builder.record(EMR_RECTANGLE, &words(&[40, 40, 50, 50]));
    let pixmap = render(&builder.finish());
    assert_eq!(rgb(&pixmap, 10, 10), [255, 0, 0]);
    assert_eq!(rgb(&pixmap, 18, 18), [255, 0, 0]);
    assert_eq!(rgb(&pixmap, 19, 19), [255, 255, 255]);
    assert_eq!(rgb(&pixmap, 40, 40), [0, 0, 0]);
    assert_eq!(rgb(&pixmap, 49, 49), [0, 0, 0]);
    assert_eq!(rgb(&pixmap, 45, 45), [255, 0, 0]);
    assert_eq!(rgb(&pixmap, 50, 50), [255, 255, 255]);
}

#[test]
fn ellipses_fill_their_box_and_not_its_corners() {
    let mut builder = Builder::new([0, 0, 2540, 2540]);
    solid_brush(&mut builder, 1, 0x00FF_0000);
    builder.record(EMR_SELECTOBJECT, &words(&[NULL_PEN as i32]));
    builder.record(EMR_ELLIPSE, &words(&[0, 0, 40, 40]));
    let pixmap = render(&builder.finish());
    assert_eq!(rgb(&pixmap, 20, 20), [0, 0, 255]);
    assert_eq!(rgb(&pixmap, 1, 1), [255, 255, 255]);
    // Not smoothed: every pixel is one colour or the other.
    let edge = rgb(&pixmap, 20, 0);
    assert!(edge == [0, 0, 255] || edge == [255, 255, 255], "{edge:?}");
}

#[test]
fn anisotropic_mapping_scales_logical_units() {
    let mut builder = Builder::new([0, 0, 2540, 2540]);
    builder.record(EMR_SETMAPMODE, &words(&[8]));
    builder.record(EMR_SETWINDOWEXTEX, &words(&[100, 100]));
    builder.record(EMR_SETVIEWPORTEXTEX, &words(&[50, 50]));
    solid_brush(&mut builder, 1, 0x0000_FF00);
    builder.record(EMR_SELECTOBJECT, &words(&[NULL_PEN as i32]));
    builder.record(EMR_RECTANGLE, &words(&[0, 0, 100, 100]));
    let pixmap = render(&builder.finish());
    assert_eq!(rgb(&pixmap, 48, 48), [0, 255, 0]);
    assert_eq!(rgb(&pixmap, 50, 50), [255, 255, 255]);
}

#[test]
fn bitmaps_stretch_into_their_destination() {
    let mut builder = Builder::new([0, 0, 2540, 2540]);
    // A 2×1 bottom-up 24-bit bitmap: red, then blue.
    let mut info = words(&[40, 2, 1]);
    info.extend(1u16.to_le_bytes());
    info.extend(24u16.to_le_bytes());
    info.extend(words(&[0, 0, 0, 0, 0, 0]));
    let bits = [0u8, 0, 255, 255, 0, 0, 0, 0];
    let info_at = 80;
    let bits_at = info_at + info.len();
    let mut payload = words(&[0, 0, 0, 0, 0, 0, 0, 0, 2, 1]);
    payload.extend(words(&[info_at as i32, info.len() as i32, bits_at as i32, bits.len() as i32, 0, 0x00CC_0020, 20, 10]));
    payload.extend(&info);
    payload.extend(bits);
    builder.record(EMR_STRETCHDIBITS, &payload);
    let pixmap = render(&builder.finish());
    assert_eq!(rgb(&pixmap, 2, 5), [255, 0, 0]);
    assert_eq!(rgb(&pixmap, 17, 5), [0, 0, 255]);
    assert_eq!(rgb(&pixmap, 2, 12), [255, 255, 255]);
}

#[test]
fn emf_plus_records_stand_in_for_the_gdi_ones() {
    let mut builder = Builder::new([0, 0, 2540, 2540]);
    let mut header = 0xDBC0_1002u32.to_le_bytes().to_vec();
    header.extend(words(&[1, 96, 96]));
    let mut fill = 0xFF00_FF00u32.to_le_bytes().to_vec();
    fill.extend(words(&[1]));
    for value in [10.0f32, 10.0, 10.0, 10.0] {
        fill.extend(value.to_le_bytes());
    }
    let page = 1.0f32.to_le_bytes().to_vec();
    builder.plus(&[(0x4001, 1, header), (0x4030, 2, page), (0x400A, 0x8000, fill)]);
    // The GDI copy of the picture, which GDI+ leaves alone.
    solid_brush(&mut builder, 1, 0x0000_00FF);
    builder.record(EMR_RECTANGLE, &words(&[50, 50, 60, 60]));
    let pixmap = render(&builder.finish());
    assert_eq!(rgb(&pixmap, 15, 15), [0, 255, 0]);
    assert_eq!(rgb(&pixmap, 55, 55), [255, 255, 255]);
    // Pixel centres sit on whole coordinates: the edge pixel is half
    // covered.
    let edge = rgb(&pixmap, 10, 15);
    assert!(edge[0] > 64 && edge[0] < 192, "{edge:?}");
}

#[test]
fn text_is_drawn_in_the_selected_font() {
    if text::find_face("Arial", 400, false, 0).is_none() {
        return;
    }
    let mut builder = Builder::new([0, 0, 2540, 2540]);
    let mut font = words(&[1, -40, 0, 0, 0, 400]);
    font.extend([0u8; 8]);
    let face: Vec<u16> = "Arial".encode_utf16().collect();
    let mut name = [0u16; 32];
    name[..face.len()].copy_from_slice(&face);
    font.extend(name.iter().flat_map(|unit| unit.to_le_bytes()));
    builder.record(EMR_EXTCREATEFONTINDIRECTW, &font);
    builder.record(EMR_SELECTOBJECT, &words(&[1]));
    // "Hi" at (10, 10), top-aligned, no rectangle.
    let text: Vec<u8> = "Hi".encode_utf16().flat_map(|unit| unit.to_le_bytes()).collect();
    let mut payload = words(&[0, 0, 0, 0, 1]);
    payload.extend(1.0f32.to_le_bytes());
    payload.extend(1.0f32.to_le_bytes());
    payload.extend(words(&[10, 10, 2, 76, 0x100, 0, 0, 0, 0, 0]));
    payload.extend(text);
    builder.record(EMR_EXTTEXTOUTW, &payload);
    let pixmap = render(&builder.finish());
    let dark = (0..60).flat_map(|y| (0..96).map(move |x| (x, y))).filter(|(x, y)| rgb(&pixmap, *x, *y)[0] < 128).count();
    assert!(dark > 30, "{dark} dark pixels");
}

#[test]
fn the_pixel_cache_is_opaque_rgba_quanta() {
    let mut builder = Builder::new([0, 0, 1270, 1270]);
    solid_brush(&mut builder, 1, 0x0000_00FF);
    builder.record(EMR_SELECTOBJECT, &words(&[NULL_PEN as i32]));
    builder.record(EMR_RECTANGLE, &words(&[0, 0, 10, 10]));
    let directory = std::env::temp_dir().join(format!("nerthus-emf-{}", std::process::id()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("square.emf");
    std::fs::write(&path, builder.finish()).unwrap();
    let cache = crate::pixel_cache::read_pixel_cache(&path).unwrap();
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!((cache.width, cache.height), (48, 48));
    assert_eq!(cache.channels, vec![PixelChannel::Red, PixelChannel::Green, PixelChannel::Blue, PixelChannel::Alpha]);
    assert_eq!(&cache.data[..4], &[65535.0, 0.0, 0.0, 65535.0]);
    let last = cache.data.len() - 4;
    assert_eq!(&cache.data[last..], &[65535.0; 4]);
}

#[test]
fn files_that_are_not_metafiles_are_refused() {
    assert!(Header::parse(b"not a metafile at all, really not").is_none());
}

/// Renders every metafile in `NERTHUS_EMF_DIR` to a PNG beside it, for
/// looking over.
#[test]
#[ignore]
fn render_folder() {
    let directory = std::env::var("NERTHUS_EMF_DIR").expect("set NERTHUS_EMF_DIR");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&directory).unwrap().flatten().map(|entry| entry.path()).collect();
    files.retain(|path| path.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("emf")));
    files.sort();
    for path in files {
        let bytes = std::fs::read(&path).unwrap();
        let started = std::time::Instant::now();
        let Some(header) = Header::parse(&bytes) else {
            eprintln!("{}: not a metafile", path.display());
            continue;
        };
        let (width, height) = header.picture_size();
        if width < 1.0 || height < 1.0 || width > MAXIMUM_SIDE || height > MAXIMUM_SIDE {
            eprintln!("{}: size {width}×{height}", path.display());
            continue;
        }
        let mut pixmap = Pixmap::new(width as u32, height as u32).unwrap();
        pixmap.fill(Color::WHITE);
        play(&bytes, &header, &mut pixmap, 0);
        pixmap.save_png(path.with_extension("png")).unwrap();
        eprintln!("{}: {}×{} in {:?}", path.file_name().unwrap().to_string_lossy(), width, height, started.elapsed());
    }
}

/// Lists the records of `NERTHUS_EMF_FILE` (and its EMF+ records).
#[test]
#[ignore]
fn dump_records() {
    let path = std::env::var("NERTHUS_EMF_FILE").expect("set NERTHUS_EMF_FILE");
    let bytes = std::fs::read(path).unwrap();
    let header = Header::parse(&bytes).unwrap();
    eprintln!("{header:?} size {:?} dpi {:?}", header.picture_size(), header.dpi());
    for record in (Records { bytes: &bytes, at: 0 }) {
        let words: Vec<i32> = (8..record.data.len().min(48)).step_by(4).map(|at| record.data.i32_at(at)).collect();
        eprintln!("{:3} size {:5} {:?}", record.kind, record.data.len(), words);
        for plus in plus_records(record.data) {
            let words: Vec<String> = (0..plus.data.len().min(40)).step_by(4).map(|at| format!("{:#x}/{}", plus.data.u32_at(at), plus.data.f32_at(at))).collect();
            eprintln!("    {:#06x} flags {:#06x} size {:5} {}", plus.kind, plus.flags, plus.data.len(), words.join(" "));
        }
    }
}
