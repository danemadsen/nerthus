//! Minimal MS-NRBF (.NET BinaryFormatter) reader, enough to load
//! `.tor.resource` files.

use std::collections::HashMap;

use anyhow::{anyhow, bail, Result};

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Bool(bool),
    Byte(u8),
    SByte(i8),
    Char(char),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Single(f32),
    Double(f64),
    String(String),
    Bytes(Vec<u8>),
    Floats(Vec<f32>),
    Doubles(Vec<f64>),
    Ints(Vec<i32>),
    Array(Vec<Value>),
    Object(Object),
    Reference(i32),
}

#[derive(Clone, Debug)]
pub struct Object {
    pub class_name: String,
    pub members: Vec<(String, Value)>,
}

impl Object {
    pub fn member(&self, name: &str) -> Option<&Value> {
        self.members.iter().find(|(member, _)| member == name || member.ends_with(&format!("<{name}>k__BackingField"))).map(|(_, value)| value)
    }
}

#[derive(Clone, Debug)]
enum MemberType {
    Primitive(u8),
    String,
    Object,
    SystemClass,
    Class,
    ObjectArray,
    StringArray,
    /// Element type; arrays carry it again inline, so the reader skips it.
    PrimitiveArray(#[allow(dead_code)] u8),
}

#[derive(Clone, Debug)]
struct ClassMeta {
    name: String,
    member_names: Vec<String>,
    member_types: Option<Vec<MemberType>>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    position: usize,
    classes: HashMap<i32, ClassMeta>,
    objects: HashMap<i32, Value>,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8> {
        let value = *self.bytes.get(self.position).ok_or_else(|| anyhow!("unexpected end of data"))?;
        self.position += 1;
        Ok(value)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        if self.position + count > self.bytes.len() {
            bail!("unexpected end of data");
        }
        let slice = &self.bytes[self.position..self.position + count];
        self.position += count;
        Ok(slice)
    }

    fn i32(&mut self) -> Result<i32> {
        let bytes = self.take(4)?;
        Ok(i32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn string(&mut self) -> Result<String> {
        let mut length = 0usize;
        let mut shift = 0;
        loop {
            let byte = self.byte()?;
            length |= ((byte & 0x7f) as usize) << shift;
            if byte & 0x80 == 0 {
                break;
            }
            shift += 7;
        }
        let bytes = self.take(length)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    fn primitive(&mut self, kind: u8) -> Result<Value> {
        let value = match kind {
            1 => Value::Bool(self.byte()? != 0),
            2 => Value::Byte(self.byte()?),
            3 => Value::Char(self.byte()? as char),
            5 => Value::String(self.string()?),
            6 => {
                let bytes = self.take(8)?;
                Value::Double(f64::from_le_bytes(bytes.try_into().unwrap()))
            }
            7 => {
                let bytes = self.take(2)?;
                Value::Int16(i16::from_le_bytes([bytes[0], bytes[1]]))
            }
            8 => Value::Int32(self.i32()?),
            9 | 12 | 13 => {
                let bytes = self.take(8)?;
                Value::Int64(i64::from_le_bytes(bytes.try_into().unwrap()))
            }
            10 => Value::SByte(self.byte()? as i8),
            11 => {
                let bytes = self.take(4)?;
                Value::Single(f32::from_le_bytes(bytes.try_into().unwrap()))
            }
            14 => {
                let bytes = self.take(2)?;
                Value::UInt16(u16::from_le_bytes([bytes[0], bytes[1]]))
            }
            15 => {
                let bytes = self.take(4)?;
                Value::UInt32(u32::from_le_bytes(bytes.try_into().unwrap()))
            }
            16 => {
                let bytes = self.take(8)?;
                Value::UInt64(u64::from_le_bytes(bytes.try_into().unwrap()))
            }
            17 => Value::Null,
            18 => Value::String(self.string()?),
            other => bail!("unsupported primitive type {other}"),
        };
        Ok(value)
    }

    fn member_types(&mut self, count: usize) -> Result<Vec<MemberType>> {
        let mut kinds = Vec::with_capacity(count);
        for _ in 0..count {
            kinds.push(self.byte()?);
        }
        let mut types = Vec::with_capacity(count);
        for kind in kinds {
            let member_type = match kind {
                0 => MemberType::Primitive(self.byte()?),
                1 => MemberType::String,
                2 => MemberType::Object,
                3 => {
                    self.string()?;
                    MemberType::SystemClass
                }
                4 => {
                    self.string()?;
                    self.i32()?;
                    MemberType::Class
                }
                5 => MemberType::ObjectArray,
                6 => MemberType::StringArray,
                7 => MemberType::PrimitiveArray(self.byte()?),
                other => bail!("unknown member type {other}"),
            };
            types.push(member_type);
        }
        Ok(types)
    }

    fn class_info(&mut self) -> Result<(i32, String, Vec<String>)> {
        let object_id = self.i32()?;
        let name = self.string()?;
        let count = self.i32()? as usize;
        let mut names = Vec::with_capacity(count);
        for _ in 0..count {
            names.push(self.string()?);
        }
        Ok((object_id, name, names))
    }

    fn read_members(&mut self, object_id: i32, meta: &ClassMeta) -> Result<Value> {
        let mut members = Vec::with_capacity(meta.member_names.len());
        for (index, name) in meta.member_names.iter().enumerate() {
            let value = match meta.member_types.as_ref().and_then(|types| types.get(index)) {
                Some(MemberType::Primitive(kind)) => {
                    let kind = *kind;
                    self.primitive(kind)?
                }
                _ => self.record()?.unwrap_or(Value::Null),
            };
            members.push((name.clone(), value));
        }
        let object = Value::Object(Object {
            class_name: meta.name.clone(),
            members,
        });
        self.objects.insert(object_id, object.clone());
        Ok(object)
    }

    /// Reads one record; `None` for MessageEnd.
    fn record(&mut self) -> Result<Option<Value>> {
        let kind = self.byte()?;
        let value = match kind {
            0 => {
                self.take(16)?;
                return self.record();
            }
            1 => {
                let object_id = self.i32()?;
                let metadata_id = self.i32()?;
                let meta = self.classes.get(&metadata_id).cloned().ok_or_else(|| anyhow!("unknown class metadata {metadata_id}"))?;
                self.classes.insert(object_id, meta.clone());
                self.read_members(object_id, &meta)?
            }
            2 | 3 => {
                let (object_id, name, member_names) = self.class_info()?;
                if kind == 3 {
                    self.i32()?;
                }
                let meta = ClassMeta { name, member_names, member_types: None };
                self.classes.insert(object_id, meta.clone());
                self.read_members(object_id, &meta)?
            }
            4 | 5 => {
                let (object_id, name, member_names) = self.class_info()?;
                let types = self.member_types(member_names.len())?;
                if kind == 5 {
                    self.i32()?;
                }
                let meta = ClassMeta {
                    name,
                    member_names,
                    member_types: Some(types),
                };
                self.classes.insert(object_id, meta.clone());
                self.read_members(object_id, &meta)?
            }
            6 => {
                let object_id = self.i32()?;
                let text = self.string()?;
                let value = Value::String(text);
                self.objects.insert(object_id, value.clone());
                value
            }
            7 => self.binary_array()?,
            8 => {
                let primitive = self.byte()?;
                self.primitive(primitive)?
            }
            9 => Value::Reference(self.i32()?),
            10 => Value::Null,
            11 => return Ok(None),
            12 => {
                self.i32()?;
                self.string()?;
                return self.record();
            }
            13 => {
                let count = self.byte()? as usize;
                Value::Array(vec![Value::Null; count])
            }
            14 => {
                let count = self.i32()? as usize;
                Value::Array(vec![Value::Null; count])
            }
            15 => {
                let object_id = self.i32()?;
                let length = self.i32()? as usize;
                let primitive = self.byte()?;
                let value = self.primitive_array(primitive, length)?;
                self.objects.insert(object_id, value.clone());
                value
            }
            16 | 17 => {
                let object_id = self.i32()?;
                let length = self.i32()? as usize;
                let mut items = Vec::with_capacity(length);
                while items.len() < length {
                    match self.record()? {
                        Some(Value::Array(nulls)) if nulls.iter().all(|item| matches!(item, Value::Null)) => items.extend(nulls),
                        Some(value) => items.push(value),
                        None => break,
                    }
                }
                let value = Value::Array(items);
                self.objects.insert(object_id, value.clone());
                value
            }
            other => bail!("unsupported record type {other} at {}", self.position - 1),
        };
        Ok(Some(value))
    }

    fn primitive_array(&mut self, primitive: u8, length: usize) -> Result<Value> {
        let value = match primitive {
            2 => Value::Bytes(self.take(length)?.to_vec()),
            11 => {
                let bytes = self.take(length * 4)?;
                let (words, _) = bytes.as_chunks::<4>();
                Value::Floats(words.iter().map(|word| f32::from_le_bytes(*word)).collect())
            }
            6 => {
                let bytes = self.take(length * 8)?;
                let (words, _) = bytes.as_chunks::<8>();
                Value::Doubles(words.iter().map(|word| f64::from_le_bytes(*word)).collect())
            }
            8 => {
                let bytes = self.take(length * 4)?;
                let (words, _) = bytes.as_chunks::<4>();
                Value::Ints(words.iter().map(|word| i32::from_le_bytes(*word)).collect())
            }
            other => {
                let mut items = Vec::with_capacity(length);
                for _ in 0..length {
                    items.push(self.primitive(other)?);
                }
                Value::Array(items)
            }
        };
        Ok(value)
    }

    fn binary_array(&mut self) -> Result<Value> {
        let object_id = self.i32()?;
        let array_type = self.byte()?;
        let rank = self.i32()? as usize;
        let mut lengths = Vec::with_capacity(rank);
        for _ in 0..rank {
            lengths.push(self.i32()? as usize);
        }
        if matches!(array_type, 3..=5) {
            for _ in 0..rank {
                self.i32()?;
            }
        }
        let member_type = self.member_types(1)?.remove(0);
        let total: usize = lengths.iter().product();
        let value = match member_type {
            MemberType::Primitive(primitive) => self.primitive_array(primitive, total)?,
            _ => {
                let mut items = Vec::with_capacity(total);
                while items.len() < total {
                    match self.record()? {
                        Some(Value::Array(nulls)) if nulls.iter().all(|item| matches!(item, Value::Null)) => items.extend(nulls),
                        Some(value) => items.push(value),
                        None => break,
                    }
                }
                Value::Array(items)
            }
        };
        self.objects.insert(object_id, value.clone());
        Ok(value)
    }
}

/// Parses a BinaryFormatter stream, returning the root value and the object table.
pub fn parse(bytes: &[u8]) -> Result<(Value, HashMap<i32, Value>)> {
    let mut reader = Reader {
        bytes,
        position: 0,
        classes: HashMap::new(),
        objects: HashMap::new(),
    };
    if reader.byte()? != 0 {
        bail!("not a BinaryFormatter stream");
    }
    let root_id = reader.i32()?;
    reader.take(12)?;
    while reader.position < bytes.len() {
        if reader.record()?.is_none() {
            break;
        }
    }
    let root = reader.objects.get(&root_id).cloned().unwrap_or(Value::Null);
    Ok((root, reader.objects))
}

/// Follows references through the object table.
pub fn resolve<'a>(value: &'a Value, objects: &'a HashMap<i32, Value>) -> &'a Value {
    match value {
        Value::Reference(id) => objects.get(id).map(|target| resolve(target, objects)).unwrap_or(&Value::Null),
        other => other,
    }
}

/// Decodes a serialized `System.Guid`.
pub fn guid(value: &Value, objects: &HashMap<i32, Value>) -> Option<uuid::Uuid> {
    let Value::Object(object) = resolve(value, objects) else { return None };
    let a = match object.member("_a")? {
        Value::Int32(value) => *value as u32,
        _ => return None,
    };
    let b = match object.member("_b")? {
        Value::Int16(value) => *value as u16,
        _ => return None,
    };
    let c = match object.member("_c")? {
        Value::Int16(value) => *value as u16,
        _ => return None,
    };
    let mut tail = [0u8; 8];
    for (index, name) in ["_d", "_e", "_f", "_g", "_h", "_i", "_j", "_k"].iter().enumerate() {
        tail[index] = match object.member(name)? {
            Value::Byte(byte) => *byte,
            _ => 0,
        };
    }
    Some(uuid::Uuid::from_fields(a, b, c, &tail))
}
