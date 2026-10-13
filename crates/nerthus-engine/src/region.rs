//! Regions (`Region`, a `WorldPortion`): square parts of the terrain that
//! a Region build builds on their own, at their own resolution, from the
//! baked maps cut to them. A terrain keeps them in `Terrain.Regions`; the
//! build definition's `Portion` is the one to build (in `.terrain` files a
//! `$ref` to it).

use serde_json::{Map as JsonObject, Value};

use crate::format_names::value_type;
use crate::json::{collection_items, f32_value, type_class_name};
use crate::project::Terrain;

/// `Region`'s own members, in declaration order (Newtonsoft writes them
/// before `WorldPortion`'s).
const REGION_MEMBERS: [&str; 6] = ["Name", "Color", "ProportionatePosition", "ProportionateWidth", "Size", "Blend"];

/// One region of a terrain.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub name: String,
    /// `Color`, as 0–1 floats.
    pub color: [f32; 3],
    /// `ProportionatePosition`: the top-left corner, as fractions of the
    /// terrain's width.
    pub position: [f32; 2],
    /// `ProportionateWidth`: the side, as a fraction of the terrain's width.
    pub width: f32,
    /// `Size`: the resolution a Region build builds it at.
    pub size: i32,
    /// `Blend`.
    pub blend: f32,
    /// The other members as read (`WorldPortion`'s, and computed ones
    /// Newtonsoft writes such as `WorldWidth`), written back after the
    /// region's own.
    pub others: JsonObject<String, Value>,
}

impl Region {
    /// `new Region { Name = name }`.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            color: [0.0; 3],
            position: [0.0; 2],
            width: 0.2,
            size: 2048,
            blend: 0.25,
            others: JsonObject::new(),
        }
    }

    /// A region as Newtonsoft reads it: missing members are zero.
    pub fn from_json(value: &Value) -> Self {
        let float = |member: Option<&Value>, name: &str| member.and_then(|value| value.get(name)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let color = value.get("Color");
        let position = value.get("ProportionatePosition");
        let mut others = JsonObject::new();
        for (name, member) in value.as_object().into_iter().flatten() {
            if name.starts_with('$') || REGION_MEMBERS.contains(&name.as_str()) {
                continue;
            }
            others.insert(name.clone(), member.clone());
        }
        Self {
            name: value.get("Name").and_then(Value::as_str).unwrap_or_default().to_string(),
            color: [float(color, "R"), float(color, "G"), float(color, "B")],
            position: [float(position, "X"), float(position, "Y")],
            width: float(Some(value), "ProportionateWidth"),
            size: value.get("Size").and_then(Value::as_i64).unwrap_or(0) as i32,
            blend: float(Some(value), "Blend"),
            others,
        }
    }

    /// The region as project files hold it: its own members in declaration order
    /// (zero ones left out; `ProportionatePosition` is an object, so it is
    /// always there), then the others as read.
    pub fn to_json(&self) -> Value {
        let mut members = JsonObject::new();
        members.insert("Name".to_string(), Value::from(self.name.clone()));
        let channels = [("R", self.color[0]), ("G", self.color[1]), ("B", self.color[2])];
        if channels.iter().any(|(_, value)| *value != 0.0) {
            members.insert("Color".to_string(), Value::Object(nonzero_floats(&channels)));
        }
        let corner = [("X", self.position[0]), ("Y", self.position[1])];
        members.insert("ProportionatePosition".to_string(), Value::Object(nonzero_floats(&corner)));
        if self.width != 0.0 {
            members.insert("ProportionateWidth".to_string(), f32_value(self.width));
        }
        if self.size != 0 {
            members.insert("Size".to_string(), Value::from(self.size));
        }
        if self.blend != 0.0 {
            members.insert("Blend".to_string(), f32_value(self.blend));
        }
        for (name, member) in &self.others {
            members.insert(name.clone(), member.clone());
        }
        Value::Object(members)
    }

    /// `WorldPortion.WorldWidth`, a computed member (the build
    /// definition's `WorldResolution`) written as it is at saving.
    pub fn set_world_width(&mut self, world_width: usize) {
        if world_width == 0 {
            self.others.shift_remove("WorldWidth");
            return;
        }
        self.others.insert("WorldWidth".to_string(), Value::from(world_width));
    }

    /// The region's window on a map `resolution` pixels wide: the
    /// left and top pixel and the side, each truncated toward zero.
    pub fn window(&self, resolution: usize) -> (i64, i64, i64) {
        let pixels = resolution as f32;
        let left = (self.position[0] * pixels) as i64;
        let top = (self.position[1] * pixels) as i64;
        let side = (self.width * pixels) as i64;
        (left, top, side)
    }
}

fn nonzero_floats(values: &[(&str, f32)]) -> JsonObject<String, Value> {
    let mut members = JsonObject::new();
    for (name, value) in values {
        if *value != 0.0 {
            members.insert(name.to_string(), f32_value(*value));
        }
    }
    members
}

/// `Terrain.Regions`.
pub fn regions(terrain: &Terrain) -> Vec<Region> {
    let Some(list) = terrain.extra.get("Regions") else {
        return Vec::new();
    };
    collection_items(list).iter().map(Region::from_json).collect()
}

/// Replaces `Terrain.Regions`.
pub fn set_regions(terrain: &mut Terrain, regions: &[Region]) {
    let list: Vec<Value> = regions.iter().map(Region::to_json).collect();
    terrain.extra.insert("Regions".to_string(), Value::Array(list));
}

/// The region a build definition's `Portion` holds, if it is one (a
/// `WorldPortion` that is not a `Region` is not).
pub fn portion(definition: &Value) -> Option<Region> {
    let portion = definition.get("Portion")?;
    let type_name = portion.get("$type").and_then(Value::as_str)?;
    if type_class_name(type_name) != "Region" {
        return None;
    }
    Some(Region::from_json(portion))
}

/// A region as `BuildDefinition.Portion` holds it (typed, as Newtonsoft
/// writes a `Region` in a `WorldPortion` member).
pub fn portion_json(region: &Region) -> Value {
    let mut members = JsonObject::new();
    members.insert("$type".to_string(), Value::from(value_type("Region")));
    if let Value::Object(region_members) = region.to_json() {
        members.extend(region_members);
    }
    Value::Object(members)
}

/// `BuildDefinition.Portion = region` (null, left out of the file, for
/// none).
pub fn set_portion(definition: &mut JsonObject<String, Value>, region: Option<&Region>) {
    let Some(region) = region else {
        definition.shift_remove("Portion");
        return;
    };
    let Value::Object(members) = portion_json(region) else {
        return;
    };
    if definition.contains_key("Portion") {
        definition.insert("Portion".to_string(), Value::Object(members));
        return;
    }
    // `Portion` follows `Type` in BuildDefinition's declaration order.
    let mut reordered = JsonObject::new();
    let mut placed = false;
    for (name, member) in std::mem::take(definition) {
        let after_type = name == "Type";
        reordered.insert(name, member);
        if after_type {
            reordered.insert("Portion".to_string(), Value::Object(members.clone()));
            placed = true;
        }
    }
    if !placed {
        reordered.insert("Portion".to_string(), Value::Object(members));
    }
    *definition = reordered;
}

/// `Terrain.Ratio`: height over width, for a region as if the region were
/// the whole terrain (`Height / Width * (Width / (Width * ProportionateWidth))`).
pub fn terrain_ratio(width: f32, height: f32, region: Option<&Region>) -> f32 {
    match region {
        Some(region) => height / width * (width / (width * region.width)),
        None => height / width,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn regions_round_trip_as_saved() {
        // From a saved project.
        let saved = json!({
            "Name": "Region 001",
            "Color": {"R": 0.5, "G": 0.25, "B": 1.0},
            "ProportionatePosition": {"X": 0.25, "Y": 0.125},
            "ProportionateWidth": 0.5,
            "Size": 1024,
            "Blend": 0.25,
            "WorldWidth": 2048
        });
        let region = Region::from_json(&saved);
        assert_eq!(region.color, [0.5, 0.25, 1.0]);
        assert_eq!(region.position, [0.25, 0.125]);
        assert_eq!(region.size, 1024);
        assert_eq!(region.to_json(), saved);

        let fresh = json!({"Name": "Second", "ProportionatePosition": {}, "ProportionateWidth": 0.2, "Size": 2048, "Blend": 0.25, "WorldWidth": 2048});
        let mut second = Region::new("Second");
        second.set_world_width(2048);
        assert_eq!(second.to_json(), fresh);
    }

    #[test]
    fn portion_goes_after_type() {
        let mut definition = JsonObject::new();
        definition.insert("Type".to_string(), json!("Region"));
        definition.insert("Resolution".to_string(), json!(2048));
        let region = Region::new("North");
        set_portion(&mut definition, Some(&region));
        let names: Vec<&str> = definition.keys().map(String::as_str).collect();
        assert_eq!(names, ["Type", "Portion", "Resolution"]);
        let read = portion(&Value::Object(definition.clone())).expect("the portion is a region");
        assert_eq!(read.name, "North");
        set_portion(&mut definition, None);
        assert!(!definition.contains_key("Portion"));
    }

    #[test]
    fn region_ratio_scales_by_its_width() {
        let mut region = Region::new("Half");
        region.width = 0.5;
        assert_eq!(terrain_ratio(5000.0, 2500.0, None), 0.5);
        assert_eq!(terrain_ratio(5000.0, 2500.0, Some(&region)), 1.0);
    }
}
