//! Project resources (`<project>.tor.resource`): painted masks, imported
//! images and other binary data, stored as a BinaryFormatter list.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use nerthus_core::ResourceProvider;
use uuid::Uuid;

use crate::nrbf::{self, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceKind {
    Ink,
    Mesh,
    Mask,
    Generic,
}

#[derive(Clone, Debug)]
pub enum ResourceData {
    Bytes(Arc<Vec<u8>>),
    Floats(Arc<Vec<f32>>),
    Other,
}

#[derive(Clone, Debug)]
pub struct ResourceEntry {
    pub id: Uuid,
    pub parent: Uuid,
    pub note: String,
    pub kind: ResourceKind,
    pub original_path: Option<String>,
    pub data: ResourceData,
}

#[derive(Clone, Debug, Default)]
pub struct ResourceStore {
    pub entries: Vec<ResourceEntry>,
    pub base_dir: Option<PathBuf>,
}

impl ResourceStore {
    pub fn new(base_dir: Option<PathBuf>) -> Self {
        Self { entries: Vec::new(), base_dir }
    }

    /// Loads `<project>.resource` next to a project if it exists.
    pub fn for_project(project_path: &Path) -> Self {
        let base_dir = project_path.parent().map(Path::to_path_buf);
        let mut store = ResourceStore::new(base_dir);
        let mut resource_path = project_path.as_os_str().to_owned();
        resource_path.push(".resource");
        let resource_path = PathBuf::from(resource_path);
        if resource_path.exists() {
            match load_resource_file(&resource_path) {
                Ok(entries) => store.entries = entries,
                Err(error) => log_warning(&format!("cannot read {}: {error}", resource_path.display())),
            }
        }
        store
    }

    pub fn find(&self, id: Uuid) -> Option<&ResourceEntry> {
        self.entries.iter().find(|entry| entry.id == id).or_else(|| self.entries.iter().find(|entry| entry.parent == id))
    }

    pub fn insert_mask(&mut self, parent: Uuid, bytes: Vec<u8>, original_path: Option<String>) -> Uuid {
        self.entries.retain(|entry| !(entry.parent == parent && entry.kind == ResourceKind::Mask));
        let id = parent;
        self.entries.push(ResourceEntry {
            id,
            parent,
            note: String::new(),
            kind: ResourceKind::Mask,
            original_path,
            data: ResourceData::Bytes(Arc::new(bytes)),
        });
        id
    }
}

fn log_warning(message: &str) {
    eprintln!("warning: {message}");
}

impl ResourceProvider for ResourceStore {
    fn image_bytes(&self, id: Uuid) -> Option<Arc<Vec<u8>>> {
        match &self.find(id)?.data {
            ResourceData::Bytes(bytes) => Some(bytes.clone()),
            _ => None,
        }
    }

    fn floats(&self, id: Uuid) -> Option<Arc<Vec<f32>>> {
        match &self.find(id)?.data {
            ResourceData::Floats(values) => Some(values.clone()),
            _ => None,
        }
    }

    fn base_dir(&self) -> Option<PathBuf> {
        self.base_dir.clone()
    }
}

fn load_resource_file(path: &Path) -> Result<Vec<ResourceEntry>> {
    let bytes = std::fs::read(path)?;
    let (root, objects) = nrbf::parse(&bytes)?;
    let mut entries = Vec::new();
    let items = match nrbf::resolve(&root, &objects) {
        Value::Object(list) => match list.member("_items").map(|items| nrbf::resolve(items, &objects)) {
            Some(Value::Array(items)) => items.clone(),
            _ => Vec::new(),
        },
        Value::Array(items) => items.clone(),
        _ => Vec::new(),
    };
    for item in &items {
        if let Some(entry) = convert_entry(item, &objects) {
            entries.push(entry);
        }
    }
    Ok(entries)
}

fn convert_entry(value: &Value, objects: &HashMap<i32, Value>) -> Option<ResourceEntry> {
    let Value::Object(object) = nrbf::resolve(value, objects) else { return None };
    let id = object.member("Id").and_then(|value| nrbf::guid(value, objects)).unwrap_or_default();
    let parent = object.member("Parent").and_then(|value| nrbf::guid(value, objects)).unwrap_or_default();
    let note = match object.member("Note").map(|value| nrbf::resolve(value, objects)) {
        Some(Value::String(text)) => text.clone(),
        _ => String::new(),
    };
    let original_path = match object.member("OriginalPath").map(|value| nrbf::resolve(value, objects)) {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    };
    let kind = if object.class_name.ends_with("MaskResource") {
        ResourceKind::Mask
    } else if object.class_name.ends_with("MeshResource") {
        ResourceKind::Mesh
    } else if object.class_name.ends_with("InkResource") {
        ResourceKind::Ink
    } else {
        ResourceKind::Generic
    };
    let data = match object.member("Data").map(|value| nrbf::resolve(value, objects)) {
        Some(Value::Bytes(bytes)) => ResourceData::Bytes(Arc::new(bytes.clone())),
        Some(Value::Floats(values)) => ResourceData::Floats(Arc::new(values.clone())),
        _ => ResourceData::Other,
    };
    Some(ResourceEntry {
        id,
        parent,
        note,
        kind,
        original_path,
        data,
    })
}
