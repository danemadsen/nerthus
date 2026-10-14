//! Assimp's PLY importer (`PlyLoader.cpp`, `PlyParser.cpp`) over
//! `IOStreamBuffer` with a 1 MiB cache.
//!
//! The header is parsed line by line in a vector that the parser erases
//! from the front and the stream buffer resizes, so stale bytes of earlier
//! lines follow each line and are read where the parser runs past a line
//! end. A `comment` line in the header reads as an element with no
//! instances. ASCII instances are parsed a line each; binary ones from
//! cache-sized blocks that run on past the file's end into the cache's
//! '\n' filling. One mesh: positions, normals, colours and texture
//! coordinates from the vertex element; faces from face and triangle-strip
//! lists (every strip triangle of an instance goes to the same face, so
//! the last one stays).

use std::collections::HashSet;

use crate::fast_atof::{fast_atoreal_move, fast_atoreal_move_f64, strtol10, strtoul10};
use crate::parsing::{at, is_space_or_newline, skip_line, skip_spaces, skip_spaces_and_line_end, starts_with_at};
use crate::scene::{ALLOCATION_LIMIT, ImportError, Mesh, PRIMITIVE_POINT, Scene, bad_alloc, fail};
use crate::stream_buffer::StreamBuffer;

/// The stream buffer's cache for PLY files.
const CACHE_SIZE: usize = 1024 * 1024;

/// `PLY::EDataType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DataType {
    Char,
    UChar,
    Short,
    UShort,
    Int,
    UInt,
    Float,
    Double,
    Invalid,
}

impl DataType {
    /// Bytes a binary value takes.
    fn size(self) -> usize {
        match self {
            DataType::Char | DataType::UChar => 1,
            DataType::Short | DataType::UShort => 2,
            DataType::Int | DataType::UInt | DataType::Float => 4,
            DataType::Double => 8,
            DataType::Invalid => 0,
        }
    }
}

/// `PLY::ESemantic`, as far as the mesh uses it; the material channels
/// are `Material`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Semantic {
    XCoord,
    YCoord,
    ZCoord,
    XNormal,
    YNormal,
    ZNormal,
    UTexture,
    VTexture,
    Red,
    Green,
    Blue,
    Alpha,
    VertexIndex,
    TextureCoordinates,
    PhongPower,
    Opacity,
    Material,
    Invalid,
}

/// `PLY::EElementSemantic`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ElementSemantic {
    Vertex,
    Face,
    TriStrip,
    Edge,
    Material,
    TextureFile,
    Invalid,
}

/// `PLY::Property`.
#[derive(Clone, Copy, Debug)]
struct Property {
    data_type: DataType,
    semantic: Semantic,
    is_list: bool,
    first_type: DataType,
}

impl Default for Property {
    fn default() -> Self {
        Property { data_type: DataType::Int, semantic: Semantic::XCoord, is_list: false, first_type: DataType::UChar }
    }
}

/// `PLY::Element`.
#[derive(Clone, Debug)]
struct Element {
    properties: Vec<Property>,
    semantic: ElementSemantic,
    name: Vec<u8>,
    count: u32,
}

/// `PropertyInstance::ValueUnion`: its eight bytes, the 32-bit members in
/// the low four.
#[derive(Clone, Copy, Debug, Default)]
struct Value(u64);

impl Value {
    fn from_u32(value: u32) -> Self {
        Value(value as u64)
    }

    fn from_i32(value: i32) -> Self {
        Value(value as u32 as u64)
    }

    fn from_f32(value: f32) -> Self {
        Value(value.to_bits() as u64)
    }

    fn from_f64(value: f64) -> Self {
        Value(value.to_bits())
    }

    fn uint(self) -> u32 {
        self.0 as u32
    }

    fn int(self) -> i32 {
        self.0 as u32 as i32
    }

    fn float(self) -> f32 {
        f32::from_bits(self.0 as u32)
    }

    fn double(self) -> f64 {
        f64::from_bits(self.0)
    }

    /// `ConvertTo<ai_real>`.
    fn to_real(self, data_type: DataType) -> f32 {
        match data_type {
            DataType::Float => self.float(),
            DataType::Double => self.double() as f32,
            DataType::UInt | DataType::UShort | DataType::UChar => self.uint() as f32,
            DataType::Int | DataType::Short | DataType::Char => self.int() as f32,
            DataType::Invalid => 0.0,
        }
    }

    /// `ConvertTo<unsigned int>`: floats through a 64-bit truncation, as
    /// x64 compilers convert them.
    fn to_uint(self, data_type: DataType) -> u32 {
        let truncate = |value: f64| -> u32 {
            if value.is_nan() || value.abs() >= 9.223_372_036_854_776e18 {
                return 0;
            }
            value as i64 as u32
        };
        match data_type {
            DataType::Float => truncate(self.float() as f64),
            DataType::Double => truncate(self.double()),
            DataType::UInt | DataType::UShort | DataType::UChar => self.uint(),
            DataType::Int | DataType::Short | DataType::Char => self.int() as u32,
            DataType::Invalid => 0,
        }
    }

    /// `ConvertTo<int>`: an out-of-range float gives `INT_MIN`.
    fn to_int(self, data_type: DataType) -> i32 {
        let truncate = |value: f64| -> i32 {
            if value.is_nan() || !(-2_147_483_648.0..2_147_483_648.0).contains(&value) {
                return i32::MIN;
            }
            value as i32
        };
        match data_type {
            DataType::Float => truncate(self.float() as f64),
            DataType::Double => truncate(self.double()),
            DataType::UInt | DataType::UShort | DataType::UChar => self.uint() as i32,
            DataType::Int | DataType::Short | DataType::Char => self.int(),
            DataType::Invalid => 0,
        }
    }
}

/// One element instance: each property's values.
type Instance = Vec<Vec<Value>>;

/// `PLY::DOM`: the header's elements and each one's instances.
type Dom = (Vec<Element>, Vec<Vec<Instance>>);

/// `PLY::DOM::SkipSpaces`: the leading blanks erased; whether the line
/// goes on.
fn dom_skip_spaces(buffer: &mut Vec<u8>) -> bool {
    if buffer.is_empty() {
        return false;
    }
    let end = buffer.len();
    let (position, goes_on) = skip_spaces(buffer, 0, end);
    buffer.drain(..position.min(end));
    goes_on
}

/// `PLY::DOM::SkipLine`.
fn dom_skip_line(buffer: &mut Vec<u8>) -> bool {
    if buffer.is_empty() {
        return false;
    }
    let end = buffer.len();
    let (position, goes_on) = skip_line(buffer, 0, end);
    buffer.drain(..position.min(end));
    goes_on
}

/// `PLY::DOM::SkipSpacesAndLineEnd`.
fn dom_skip_spaces_and_line_end(buffer: &mut Vec<u8>) -> bool {
    if buffer.is_empty() {
        return false;
    }
    let end = buffer.len();
    let (position, goes_on) = skip_spaces_and_line_end(buffer, 0, end);
    buffer.drain(..position.min(end));
    goes_on
}

/// `TokenMatch` at `index`: the token and a blank or line end after it;
/// the position after the blank (or at the NUL).
fn token_match_at(buffer: &[u8], index: usize, token: &[u8]) -> Option<usize> {
    let after = at(buffer, index + token.len());
    if !starts_with_at(buffer, index, token) || !is_space_or_newline(after) {
        return None;
    }
    Some(if after != 0 { index + token.len() + 1 } else { index + token.len() })
}

/// `PLY::DOM::TokenMatch`: the token erased with the blank after it.
fn dom_token_match(buffer: &mut Vec<u8>, token: &[u8]) -> bool {
    if buffer.is_empty() {
        return false;
    }
    let Some(position) = token_match_at(buffer, 0, token) else {
        return false;
    };
    buffer.drain(..position.min(buffer.len()));
    true
}

/// `PLY::DOM::SkipComments` (it works on a copy): whether the line is a
/// comment.
fn dom_is_comment(buffer: &[u8]) -> bool {
    if buffer.is_empty() {
        return false;
    }
    let (position, goes_on) = skip_spaces(buffer, 0, buffer.len());
    goes_on && token_match_at(buffer, position, b"comment").is_some()
}

/// The bytes up to the first NUL (`strlen`).
fn c_string(buffer: &[u8]) -> &[u8] {
    let length = buffer.iter().position(|&byte| byte == 0).unwrap_or(buffer.len());
    &buffer[..length]
}

/// `Property::ParseDataType`.
fn parse_data_type(buffer: &mut Vec<u8>) -> DataType {
    let options: [(&[&[u8]], DataType); 8] = [
        (&[b"char", b"int8"], DataType::Char),
        (&[b"uchar", b"uint8"], DataType::UChar),
        (&[b"short", b"int16"], DataType::Short),
        (&[b"ushort", b"uint16"], DataType::UShort),
        (&[b"int32", b"int"], DataType::Int),
        (&[b"uint32", b"uint"], DataType::UInt),
        (&[b"float", b"float32"], DataType::Float),
        (&[b"double64", b"double", b"float64"], DataType::Double),
    ];
    for (tokens, data_type) in options {
        if tokens.iter().any(|token| dom_token_match(buffer, token)) {
            return data_type;
        }
    }
    DataType::Invalid
}

/// `Property::ParseSemantic`: the names tried in order (the material
/// channels all as `Material`); an unknown name skips the line.
fn parse_property_semantic(buffer: &mut Vec<u8>) -> Semantic {
    const NAMES: [(&[u8], Semantic); 40] = [
        (b"red", Semantic::Red),
        (b"green", Semantic::Green),
        (b"blue", Semantic::Blue),
        (b"alpha", Semantic::Alpha),
        (b"vertex_index", Semantic::VertexIndex),
        (b"vertex_indices", Semantic::VertexIndex),
        (b"texcoord", Semantic::TextureCoordinates),
        (b"material_index", Semantic::Material),
        (b"ambient_red", Semantic::Material),
        (b"ambient_green", Semantic::Material),
        (b"ambient_blue", Semantic::Material),
        (b"ambient_alpha", Semantic::Material),
        (b"diffuse_red", Semantic::Material),
        (b"diffuse_green", Semantic::Material),
        (b"diffuse_blue", Semantic::Material),
        (b"diffuse_alpha", Semantic::Material),
        (b"specular_red", Semantic::Material),
        (b"specular_green", Semantic::Material),
        (b"specular_blue", Semantic::Material),
        (b"specular_alpha", Semantic::Material),
        (b"opacity", Semantic::Opacity),
        (b"specular_power", Semantic::PhongPower),
        (b"r", Semantic::Red),
        (b"g", Semantic::Green),
        (b"b", Semantic::Blue),
        (b"u", Semantic::UTexture),
        (b"s", Semantic::UTexture),
        (b"tx", Semantic::UTexture),
        (b"texture_u", Semantic::UTexture),
        (b"v", Semantic::VTexture),
        (b"t", Semantic::VTexture),
        (b"ty", Semantic::VTexture),
        (b"texture_v", Semantic::VTexture),
        (b"x", Semantic::XCoord),
        (b"y", Semantic::YCoord),
        (b"z", Semantic::ZCoord),
        (b"nx", Semantic::XNormal),
        (b"ny", Semantic::YNormal),
        (b"nz", Semantic::ZNormal),
        (b"", Semantic::Invalid),
    ];
    for (name, semantic) in &NAMES[..NAMES.len() - 1] {
        if dom_token_match(buffer, name) {
            return *semantic;
        }
    }
    dom_skip_line(buffer);
    Semantic::Invalid
}

/// `Property::ParseProperty`: "property TYPE NAME" or "property list
/// COUNTTYPE TYPE NAME".
fn parse_property(buffer: &mut Vec<u8>, property: &mut Property) -> bool {
    if !dom_skip_spaces(buffer) || !dom_token_match(buffer, b"property") || !dom_skip_spaces(buffer) {
        return false;
    }
    if dom_token_match(buffer, b"list") {
        property.is_list = true;
        property.first_type = parse_data_type(buffer);
        if property.first_type == DataType::Invalid {
            dom_skip_line(buffer);
            return false;
        }
        if !dom_skip_spaces(buffer) {
            return false;
        }
    }
    property.data_type = parse_data_type(buffer);
    if property.data_type == DataType::Invalid {
        dom_skip_line(buffer);
        return false;
    }
    if !dom_skip_spaces(buffer) {
        return false;
    }
    property.semantic = parse_property_semantic(buffer);
    dom_skip_spaces_and_line_end(buffer);
    true
}

/// `Element::ParseSemantic`.
fn parse_element_semantic(buffer: &mut Vec<u8>) -> ElementSemantic {
    let options: [(&[u8], ElementSemantic); 6] = [
        (b"vertex", ElementSemantic::Vertex),
        (b"face", ElementSemantic::Face),
        (b"tristrips", ElementSemantic::TriStrip),
        (b"edge", ElementSemantic::Edge),
        (b"material", ElementSemantic::Material),
        (b"TextureFile", ElementSemantic::TextureFile),
    ];
    for (token, semantic) in options {
        if dom_token_match(buffer, token) {
            return semantic;
        }
    }
    ElementSemantic::Invalid
}

/// `to_string(EElementSemantic)`, the name a nameless element is known by
/// (a face is "invalid").
fn semantic_name(semantic: ElementSemantic) -> &'static [u8] {
    match semantic {
        ElementSemantic::Vertex => b"vertex",
        ElementSemantic::TriStrip => b"tristrips",
        ElementSemantic::Edge => b"edge",
        ElementSemantic::Material => b"material",
        ElementSemantic::TextureFile => b"TextureFile",
        ElementSemantic::Face | ElementSemantic::Invalid => b"invalid",
    }
}

/// `Element::ParseElement`: "element NAME COUNT" (or "comment …"), then
/// its property lines. None when the line is no element.
fn parse_element(stream: &mut StreamBuffer, buffer: &mut Vec<u8>) -> Result<Option<Element>, ImportError> {
    let mut element = Element { properties: Vec::new(), semantic: ElementSemantic::Invalid, name: Vec::new(), count: 0 };
    if !dom_skip_spaces(buffer) {
        return Ok(None);
    }
    if !dom_token_match(buffer, b"element") && !dom_token_match(buffer, b"comment") {
        return Ok(None);
    }
    if !dom_skip_spaces(buffer) {
        return Ok(None);
    }
    element.semantic = parse_element_semantic(buffer);
    if element.semantic == ElementSemantic::Invalid {
        let text = c_string(buffer);
        let Some(last_space) = text.iter().rposition(|&byte| byte == b' ') else {
            // `std::string::erase(npos)` throws `out_of_range`.
            return fail("Internal error");
        };
        element.name = text[..last_space].to_vec();
    }
    if !dom_skip_spaces(buffer) {
        return Ok(None);
    }
    if element.semantic == ElementSemantic::TextureFile {
        let text = c_string(buffer);
        element.name = text[..text.len().saturating_sub(1)].to_vec();
        dom_skip_spaces_and_line_end(buffer);
        return Ok(Some(element));
    }
    element.count = strtoul10(buffer, 0).0;
    dom_skip_spaces_and_line_end(buffer);
    loop {
        stream.next_line(buffer);
        if dom_is_comment(buffer) {
            continue;
        }
        let mut property = Property::default();
        if !parse_property(buffer, &mut property) {
            break;
        }
        element.properties.push(property);
    }
    Ok(Some(element))
}

/// `DOM::ParseHeader`: the elements up to `end_header`; an element name
/// given twice fails. False when the file ends first.
fn parse_header(stream: &mut StreamBuffer, buffer: &mut Vec<u8>, binary: bool) -> Result<Option<Vec<Element>>, ImportError> {
    let mut elements = Vec::new();
    let mut defined: HashSet<Vec<u8>> = HashSet::new();
    while !buffer.is_empty() {
        if let Some(element) = parse_element(stream, buffer)? {
            let name = if element.name.is_empty() { semantic_name(element.semantic).to_vec() } else { element.name.clone() };
            if !defined.insert(name.clone()) {
                return fail(format!("Property '{}' in header already defined ", String::from_utf8_lossy(&name)));
            }
            elements.push(element);
        } else if dom_token_match(buffer, b"end_header") {
            break;
        } else if !stream.next_line(buffer) {
            return Ok(None);
        }
    }
    if !binary {
        dom_skip_spaces_and_line_end(buffer);
    }
    Ok(Some(elements))
}

/// `PropertyInstance::DefaultValue`: zero of any type.
fn default_value() -> Value {
    Value::default()
}

/// `PropertyInstance::ParseValue`.
fn parse_value(buffer: &[u8], position: &mut usize, data_type: DataType) -> Result<Value, ImportError> {
    let value = match data_type {
        DataType::UInt | DataType::UShort | DataType::UChar => {
            let (value, next) = strtoul10(buffer, *position);
            *position = next;
            Value::from_u32(value)
        }
        DataType::Int | DataType::Short | DataType::Char => {
            let (value, next) = strtol10(buffer, *position);
            *position = next;
            Value::from_i32(value)
        }
        DataType::Float => {
            let (value, next) = fast_atoreal_move(buffer, *position, true)?;
            *position = next;
            Value::from_f32(value)
        }
        DataType::Double => {
            let (value, next) = fast_atoreal_move_f64(buffer, *position, true)?;
            *position = next;
            Value::from_f64(value)
        }
        DataType::Invalid => Value::default(),
    };
    Ok(value)
}

/// `PropertyInstance::ParseInstance`: false when the line ended first
/// (a list keeps the values it was sized to).
fn parse_property_instance(buffer: &[u8], position: &mut usize, property: &Property, values: &mut Vec<Value>) -> Result<bool, ImportError> {
    let end = buffer.len();
    let (next, goes_on) = skip_spaces(buffer, *position, end);
    *position = next;
    if !goes_on {
        return Ok(false);
    }
    if property.is_list {
        let count_value = parse_value(buffer, position, property.first_type)?;
        let count = count_value.to_uint(property.first_type);
        if count as u64 * 8 > ALLOCATION_LIMIT {
            return bad_alloc();
        }
        values.resize(count as usize, Value::default());
        for value in values.iter_mut() {
            let (next, goes_on) = skip_spaces(buffer, *position, end);
            *position = next;
            if !goes_on {
                return Ok(false);
            }
            *value = parse_value(buffer, position, property.data_type)?;
        }
    } else {
        let value = parse_value(buffer, position, property.data_type)?;
        values.push(value);
    }
    *position = skip_spaces_and_line_end(buffer, *position, end).0;
    Ok(true)
}

/// `ElementInstance::ParseInstance`: a property that fails gets a zero
/// more.
fn parse_element_instance(buffer: &[u8], element: &Element) -> Result<Instance, ImportError> {
    let mut position = 0usize;
    let mut instance = Vec::with_capacity(element.properties.len());
    for property in &element.properties {
        let mut values = Vec::new();
        if !parse_property_instance(buffer, &mut position, property, &mut values)? {
            values.push(default_value());
        }
        instance.push(values);
    }
    Ok(instance)
}

/// The binary data being read: the block, where reading is, and how much
/// of it is left (`buffer`, `pCur`, `bufferSize`).
struct BinaryReader {
    buffer: Vec<u8>,
    position: usize,
    remaining: usize,
}

impl BinaryReader {
    /// `ParseValueBinary`'s read: the next block appended when this one
    /// runs short.
    fn take(&mut self, stream: &mut StreamBuffer, size: usize) -> Result<&[u8], ImportError> {
        if self.remaining < size {
            let mut next = Vec::new();
            if !stream.next_block(&mut next) {
                return fail("Invalid .ply file: File corrupted");
            }
            let mut joined = self.buffer[self.buffer.len() - self.remaining..].to_vec();
            joined.extend_from_slice(&next);
            self.remaining = joined.len();
            self.buffer = joined;
            self.position = 0;
        }
        let start = self.position;
        self.position += size;
        self.remaining -= size;
        Ok(&self.buffer[start..start + size])
    }

    /// `ParseValueBinary`.
    fn value(&mut self, stream: &mut StreamBuffer, data_type: DataType, big_endian: bool) -> Result<Value, ImportError> {
        let size = data_type.size();
        let bytes = self.take(stream, size)?;
        let mut array = [0u8; 8];
        array[..size].copy_from_slice(bytes);
        if big_endian {
            array[..size].reverse();
        }
        let value = match data_type {
            DataType::UChar => Value::from_u32(array[0] as u32),
            DataType::Char => Value::from_i32(array[0] as i8 as i32),
            DataType::UShort => Value::from_u32(u16::from_le_bytes([array[0], array[1]]) as u32),
            DataType::Short => Value::from_i32(i16::from_le_bytes([array[0], array[1]]) as i32),
            DataType::UInt => Value::from_u32(u32::from_le_bytes([array[0], array[1], array[2], array[3]])),
            DataType::Int => Value::from_i32(i32::from_le_bytes([array[0], array[1], array[2], array[3]])),
            DataType::Float => Value::from_f32(f32::from_le_bytes([array[0], array[1], array[2], array[3]])),
            DataType::Double => Value::from_f64(f64::from_le_bytes(array)),
            DataType::Invalid => Value::default(),
        };
        Ok(value)
    }

    /// The most bytes reads could still get: this block's rest and every
    /// block left in the file, each as long as the cache.
    fn available(&self, stream: &StreamBuffer) -> u64 {
        self.remaining as u64 + stream.blocks_left() as u64 * CACHE_SIZE as u64
    }
}

/// `ElementInstance::ParseInstanceBinary`.
fn parse_element_instance_binary(stream: &mut StreamBuffer, reader: &mut BinaryReader, element: &Element, big_endian: bool) -> Result<Instance, ImportError> {
    let mut instance = Vec::with_capacity(element.properties.len());
    for property in &element.properties {
        let mut values = Vec::new();
        if property.is_list {
            let count_value = reader.value(stream, property.first_type, big_endian)?;
            let count = count_value.to_uint(property.first_type);
            let needed = count as u64 * property.data_type.size() as u64;
            if count as u64 * 8 > ALLOCATION_LIMIT {
                return bad_alloc();
            }
            if needed > reader.available(stream) {
                return fail("Invalid .ply file: File corrupted");
            }
            values.reserve(count as usize);
            for _ in 0..count {
                values.push(reader.value(stream, property.data_type, big_endian)?);
            }
        } else {
            values.push(reader.value(stream, property.data_type, big_endian)?);
        }
        instance.push(values);
    }
    Ok(instance)
}

/// The mesh the vertex and face elements build (`mGeneratedMesh`).
#[derive(Default)]
struct Builder {
    mesh: Option<Mesh>,
}

/// `NormalizeColorValue`.
fn normalize_colour(value: Value, data_type: DataType) -> f32 {
    match data_type {
        DataType::Float => value.float(),
        DataType::Double => value.double() as f32,
        DataType::UChar => value.uint() as f32 / 255.0,
        DataType::Char => (value.int().wrapping_add(127)) as f32 / 255.0,
        DataType::UShort => value.uint() as f32 / 65535.0,
        DataType::Short => (value.int().wrapping_add(32767)) as f32 / 65535.0,
        DataType::UInt => value.uint() as f32 / 65535.0,
        DataType::Int => (value.int() as f32 / 255.0) + 0.5,
        DataType::Invalid => 0.0,
    }
}

impl Builder {
    /// `LoadVertex`.
    fn load_vertex(&mut self, element: &Element, instance: &Instance, index: u32) -> Result<(), ImportError> {
        let mut found = [None; 13];
        let mut types = [DataType::Char; 13];
        let mut count = 0;
        for (position, property) in element.properties.iter().enumerate() {
            if property.is_list {
                continue;
            }
            let slot = match property.semantic {
                Semantic::XCoord => 0,
                Semantic::YCoord => 1,
                Semantic::ZCoord => 2,
                Semantic::XNormal => 3,
                Semantic::YNormal => 4,
                Semantic::ZNormal => 5,
                Semantic::Red => 6,
                Semantic::Green => 7,
                Semantic::Blue => 8,
                Semantic::Alpha => 9,
                Semantic::UTexture => 10,
                Semantic::VTexture => 11,
                _ => continue,
            };
            count += 1;
            found[slot] = Some(position);
            types[slot] = property.data_type;
        }
        if count == 0 {
            return Ok(());
        }
        let first = |slot: usize| found[slot].map(|position: usize| instance[position][0]);
        let real = |slot: usize| first(slot).map(|value| value.to_real(types[slot]));
        let position = [real(0).unwrap_or(0.0), real(1).unwrap_or(0.0), real(2).unwrap_or(0.0)];
        let have_normal = (3..6).any(|slot| found[slot].is_some());
        let normal = [real(3).unwrap_or(0.0), real(4).unwrap_or(0.0), real(5).unwrap_or(0.0)];
        let colour_of = |slot: usize| first(slot).map(|value| normalize_colour(value, types[slot]));
        let have_colour = (6..10).any(|slot| found[slot].is_some());
        let colour = [colour_of(6).unwrap_or(0.0), colour_of(7).unwrap_or(0.0), colour_of(8).unwrap_or(0.0), colour_of(9).unwrap_or(1.0)];
        let have_coordinates = (10..12).any(|slot| found[slot].is_some());
        let coordinates = [real(10).unwrap_or(0.0), real(11).unwrap_or(0.0), 0.0];

        let vertex_count = element.count as usize;
        if self.mesh.is_none() {
            if element.count as u64 * 12 > ALLOCATION_LIMIT {
                return bad_alloc();
            }
            let mesh = Mesh { vertex_count, vertices: Some(vec![[0.0; 3]; vertex_count]), ..Mesh::default() };
            self.mesh = Some(mesh);
        }
        let mesh = self.mesh.as_mut().unwrap();
        let index = index as usize;
        if index >= mesh.vertex_count {
            return fail("Invalid .ply file: Too many vertices");
        }
        let size = mesh.vertex_count;
        mesh.vertices.as_mut().unwrap()[index] = position;
        if have_normal {
            mesh.normals.get_or_insert_with(|| vec![[0.0; 3]; size])[index] = normal;
        }
        if have_colour {
            mesh.colors[0].get_or_insert_with(|| vec![[0.0; 4]; size])[index] = colour;
        }
        if have_coordinates {
            if mesh.texture_coords[0].is_none() {
                mesh.uv_components[0] = 2;
            }
            mesh.texture_coords[0].get_or_insert_with(|| vec![[0.0; 3]; size])[index] = coordinates;
        }
        Ok(())
    }

    /// `LoadFace`, for face and triangle-strip elements.
    fn load_face(&mut self, element: &Element, instance: &Instance, index: u32) -> Result<(), ImportError> {
        let Some(mesh) = self.mesh.as_mut() else {
            return fail("Invalid .ply file: Vertices should be declared before faces");
        };
        let mut indices_property = None;
        let mut coordinates_property = None;
        let mut index_type = DataType::Char;
        let mut coordinate_type = DataType::Char;
        let strip = element.semantic == ElementSemantic::TriStrip;
        if element.semantic == ElementSemantic::Face {
            for (position, property) in element.properties.iter().enumerate() {
                if !property.is_list {
                    continue;
                }
                if property.semantic == Semantic::VertexIndex {
                    indices_property = Some(position);
                    index_type = property.data_type;
                } else if property.semantic == Semantic::TextureCoordinates {
                    coordinates_property = Some(position);
                    coordinate_type = property.data_type;
                }
            }
        } else if strip
            && let Some(position) = element.properties.iter().position(|property| property.is_list)
        {
            indices_property = Some(position);
            index_type = element.properties[position].data_type;
        }
        if indices_property.is_none() && coordinates_property.is_none() {
            return Ok(());
        }
        let face_count = element.count as usize;
        match &mesh.faces {
            None => {
                if element.count as u64 * 16 > ALLOCATION_LIMIT {
                    return bad_alloc();
                }
                mesh.faces = Some(vec![Vec::new(); face_count]);
            }
            Some(faces) => {
                if faces.len() < face_count {
                    return fail("Invalid .ply file: Too many faces");
                }
            }
        }
        let vertex_count = mesh.vertex_count;
        let faces = mesh.faces.as_mut().unwrap();
        let index = index as usize;
        if !strip {
            if let Some(property) = indices_property {
                faces[index] = instance[property].iter().map(|value| value.to_uint(index_type)).collect();
            }
            if let Some(property) = coordinates_property {
                let values = &instance[property];
                if values.len() / 3 == 2 {
                    for (offset, value) in values.iter().enumerate() {
                        // An index past the face's own reads past its array.
                        let Some(&vertex) = faces[index].get(offset / 2) else {
                            return fail("Invalid .ply file: texture coordinates without vertex indices");
                        };
                        let vertex = vertex as usize;
                        if vertex >= vertex_count {
                            continue;
                        }
                        if mesh.texture_coords[0].is_none() {
                            mesh.uv_components[0] = 2;
                        }
                        let coordinates = mesh.texture_coords[0].get_or_insert_with(|| vec![[0.0; 3]; vertex_count]);
                        coordinates[vertex][offset % 2] = value.to_real(coordinate_type);
                        coordinates[vertex][2] = 0.0;
                    }
                }
            }
            return Ok(());
        }
        let Some(property) = indices_property else {
            return Ok(());
        };
        let mut table = [-1i32, -1];
        let mut flip = false;
        for value in &instance[property] {
            let vertex = value.to_int(index_type);
            if vertex == -1 {
                table = [-1, -1];
                flip = false;
                continue;
            }
            if table[0] == -1 {
                table[0] = vertex;
                continue;
            }
            if table[1] == -1 {
                table[1] = vertex;
                continue;
            }
            let mut triangle = vec![table[0] as u32, table[1] as u32, vertex as u32];
            flip = !flip;
            if flip {
                triangle.swap(0, 1);
            }
            faces[index] = triangle;
            table = [table[1], vertex];
        }
        Ok(())
    }
}

/// The elements' instances read and handed to the builder (vertices,
/// faces, strips) or kept (the rest). `instances` gives the next
/// instance of an element.
fn build(elements: &[Element], mut instances: impl FnMut(&Element) -> Result<Instance, ImportError>, builder: &mut Builder) -> Result<Vec<Vec<Instance>>, ImportError> {
    let mut kept = Vec::with_capacity(elements.len());
    for element in elements {
        let mut element_instances = Vec::new();
        let semantic = element.semantic;
        let built = matches!(semantic, ElementSemantic::Vertex | ElementSemantic::Face | ElementSemantic::TriStrip);
        if !built && element.count as u64 * 24 > ALLOCATION_LIMIT {
            return bad_alloc();
        }
        for index in 0..element.count {
            let instance = instances(element)?;
            match semantic {
                ElementSemantic::Vertex => builder.load_vertex(element, &instance, index)?,
                ElementSemantic::Face | ElementSemantic::TriStrip => builder.load_face(element, &instance, index)?,
                _ => {
                    if semantic == ElementSemantic::Material {
                        element_instances.push(instance);
                    }
                }
            }
        }
        kept.push(element_instances);
    }
    Ok(kept)
}

/// `DOM::ParseInstance` (ASCII) and the builder.
fn parse_ascii(stream: &mut StreamBuffer, builder: &mut Builder) -> Result<Option<Dom>, ImportError> {
    let mut buffer = Vec::new();
    stream.next_line(&mut buffer);
    let Some(elements) = parse_header(stream, &mut buffer, false)? else {
        return Ok(None);
    };
    stream.next_line(&mut buffer);
    let mut kept = Vec::with_capacity(elements.len());
    for element in &elements {
        let skip_lines = element.semantic == ElementSemantic::Invalid || element.properties.is_empty();
        let mut instances = Vec::new();
        let single = std::slice::from_ref(element);
        if skip_lines {
            if element.count as u64 * 24 > ALLOCATION_LIMIT {
                return bad_alloc();
            }
            for _ in 0..element.count {
                dom_skip_line(&mut buffer);
                stream.next_line(&mut buffer);
            }
            kept.push(instances);
            continue;
        }
        let parsed = build(single, |element| {
            let instance = parse_element_instance(&buffer, element)?;
            stream.next_line(&mut buffer);
            Ok(instance)
        }, builder)?;
        instances = parsed.into_iter().next().unwrap_or_default();
        kept.push(instances);
    }
    Ok(Some((elements, kept)))
}

/// `DOM::ParseInstanceBinary` and the builder.
fn parse_binary(stream: &mut StreamBuffer, builder: &mut Builder, big_endian: bool) -> Result<Option<Dom>, ImportError> {
    let mut buffer = Vec::new();
    stream.next_line(&mut buffer);
    let Some(elements) = parse_header(stream, &mut buffer, true)? else {
        return Ok(None);
    };
    stream.next_block(&mut buffer);
    let remaining = buffer.len();
    let mut reader = BinaryReader { buffer, position: 0, remaining };
    let kept = build(&elements, |element| parse_element_instance_binary(stream, &mut reader, element, big_endian), builder)?;
    Ok(Some((elements, kept)))
}

/// `LoadMaterial`'s one way to fail: the first material element's opacity
/// is read through the specular power's index, so opacity without
/// specular power reads an index out of range.
fn check_materials(elements: &[Element], kept: &[Vec<Instance>]) -> Result<(), ImportError> {
    let Some(position) = elements.iter().position(|element| element.semantic == ElementSemantic::Material) else {
        return Ok(());
    };
    let element = &elements[position];
    let mut phong = None;
    let mut opacity = None;
    for (index, property) in element.properties.iter().enumerate() {
        if property.is_list {
            continue;
        }
        if property.semantic == Semantic::PhongPower {
            phong = Some(index);
        }
        if property.semantic == Semantic::Opacity {
            opacity = Some(index);
        }
    }
    if opacity.is_some() && phong.is_none() && !kept[position].is_empty() {
        return fail("Invalid .ply file: Property index is out of range.");
    }
    Ok(())
}

/// `PLYImporter::InternReadFile`.
pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    let Some(mut stream) = StreamBuffer::open(bytes, CACHE_SIZE) else {
        return fail("File is empty.");
    };
    let mut header_check = Vec::new();
    stream.next_line(&mut header_check);
    let magic = header_check.len() >= 3 && header_check[0].eq_ignore_ascii_case(&b'p') && header_check[1].eq_ignore_ascii_case(&b'l') && header_check[2].eq_ignore_ascii_case(&b'y');
    if !magic {
        return fail("Invalid .ply file: Incorrect magic number (expected 'ply' or 'PLY').");
    }
    let mut format_line = Vec::new();
    stream.next_line(&mut format_line);
    let end = format_line.len();
    let mut position = skip_spaces_and_line_end(&format_line, 0, end).0;
    let Some(after_format) = token_match_at(&format_line, position, b"format") else {
        return fail("Invalid .ply file: Missing format specification");
    };
    position = after_format;
    let mut builder = Builder::default();
    let parsed = if token_match_at(&format_line, position, b"ascii").is_some() {
        parse_ascii(&mut stream, &mut builder)?
    } else if starts_with_at(&format_line, position, b"binary_") {
        let big_endian = matches!(at(&format_line, position + 7), b'b' | b'B');
        parse_binary(&mut stream, &mut builder, big_endian)?
    } else {
        return fail("Invalid .ply file: Unknown file format");
    };
    let Some((elements, kept)) = parsed else {
        return fail("Invalid .ply file: Unable to build DOM");
    };
    let Some(mut mesh) = builder.mesh else {
        return fail("Invalid .ply file: Unable to extract mesh data ");
    };
    if mesh.faces.is_none() {
        mesh.primitive_types = PRIMITIVE_POINT;
    }
    check_materials(&elements, &kept)?;
    Ok(Scene { meshes: vec![mesh] })
}
