//! Groups (`Terrain.Groups`, `Group`): a framed area of a graph tab
//! that owns nodes and notes (`Children`), with a name, a swatch colour, an
//! icon, a lock and a collapsed state. A node's group is not saved with the
//! node (`Node.Group` is `[JsonIgnore]`); the groups' children say it.

use serde_json::{json, Map, Value};

use super::Document;

/// `GraphControl.GroupMinWidth` and `GroupMinHeight`.
pub const GROUP_MIN_SIZE: [f32; 2] = [270.0, 270.0];
/// A collapsed group's box (the `Collapsed` trigger: 60 by 60).
pub const COLLAPSED_SIZE: f32 = 60.0;
/// `ElementHelper.IconsList`: the icons a group may wear.
pub const GROUP_ICONS: [&str; 13] = [
    "group-code",
    "group-experiment",
    "group-flag",
    "group-help",
    "group-info",
    "group-mountain",
    "group-palette",
    "group-rain",
    "group-script",
    "group-sun",
    "group-tree",
    "group-wand",
    "group-warning",
];

/// `Group`'s members in declaration order (how Newtonsoft writes them).
const GROUP_MEMBERS: [&str; 11] = ["Id", "Name", "Description", "Color", "Icon", "Children", "IsLocked", "IsCollapsed", "Position", "Size", "GraphIndex"];

/// A group as the editor uses it.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupData {
    pub id: i32,
    /// `Name`; a group saved without one reads "Group" (`OnApplyTemplate`).
    pub name: String,
    /// `Color`: a swatch name (`Gray` when the file has none).
    pub color: String,
    /// `Icon`: a `group-*` icon, empty for the default square.
    pub icon: String,
    /// `Children`: the ids of its nodes and notes.
    pub children: Vec<i32>,
    pub locked: bool,
    pub collapsed: bool,
    pub position: [f32; 2],
    pub size: [f32; 2],
    pub graph_index: usize,
}

impl GroupData {
    /// The box the group takes on its tab (`GetBoundingBox`): 60 square
    /// while collapsed.
    pub fn bounds(&self) -> egui::Rect {
        let origin = egui::Pos2::new(self.position[0], self.position[1]);
        let size = if self.collapsed { egui::Vec2::splat(COLLAPSED_SIZE) } else { egui::Vec2::new(self.size[0], self.size[1]) };
        egui::Rect::from_min_size(origin, size)
    }

    /// `Rect.Contains(point)`: edges included.
    pub fn contains(&self, point: [f32; 2]) -> bool {
        let bounds = self.bounds();
        let inside_x = point[0] >= bounds.left() && point[0] <= bounds.right();
        let inside_y = point[1] >= bounds.top() && point[1] <= bounds.bottom();
        inside_x && inside_y
    }

    fn from_json(id: i32, value: &Value) -> Self {
        let text = |name: &str| value.get(name).and_then(Value::as_str).map(str::to_string);
        let flag = |name: &str| value.get(name).and_then(Value::as_bool).unwrap_or(false);
        let vector = |name: &str| {
            let member = value.get(name);
            let component = |axis: &str| member.and_then(|member| member.get(axis)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
            [component("X"), component("Y")]
        };
        // `Children` as Newtonsoft writes a list (`$values`), or plain.
        let children_value = value.get("Children");
        let children_list = children_value.and_then(|children| children.get("$values").or(Some(children)));
        let children_items = children_list.and_then(Value::as_array);
        let children = children_items.map(|items| items.iter().filter_map(Value::as_i64).map(|child| child as i32).collect()).unwrap_or_default();
        Self {
            id,
            name: text("Name").unwrap_or_else(|| "Group".to_string()),
            color: text("Color").unwrap_or_else(|| "Gray".to_string()),
            icon: text("Icon").unwrap_or_default(),
            children,
            locked: flag("IsLocked"),
            collapsed: flag("IsCollapsed"),
            position: vector("Position"),
            size: vector("Size"),
            graph_index: value.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize,
        }
    }

    /// As projects save it: members in declaration order, those at their
    /// default left out (`IgnoreAndPopulate`), and whatever else the file
    /// had kept after them.
    fn to_json(&self, previous: Option<&Map<String, Value>>) -> Value {
        let mut members = Map::new();
        for name in GROUP_MEMBERS {
            match name {
                "Id" => {
                    members.insert(name.to_string(), json!(self.id));
                }
                "Name" => {
                    members.insert(name.to_string(), json!(self.name));
                }
                "Color" => {
                    members.insert(name.to_string(), json!(self.color));
                }
                "Icon" => {
                    if !self.icon.is_empty() {
                        members.insert(name.to_string(), json!(self.icon));
                    }
                }
                "Children" => {
                    let children: Vec<Value> = self.children.iter().map(|child| json!(child)).collect();
                    members.insert(name.to_string(), Value::Array(children));
                }
                "IsLocked" => {
                    if self.locked {
                        members.insert(name.to_string(), json!(true));
                    }
                }
                "IsCollapsed" => {
                    if self.collapsed {
                        members.insert(name.to_string(), json!(true));
                    }
                }
                "Position" | "Size" => {
                    let vector = if name == "Position" { self.position } else { self.size };
                    if vector != [0.0, 0.0] {
                        let x = nerthus_engine::json::f32_value(vector[0]);
                        let y = nerthus_engine::json::f32_value(vector[1]);
                        members.insert(name.to_string(), json!({"X": x, "Y": y}));
                    }
                }
                "GraphIndex" => {
                    if self.graph_index != 0 {
                        members.insert(name.to_string(), json!(self.graph_index));
                    }
                }
                _ => {
                    let kept = previous.and_then(|previous| previous.get(name));
                    if let Some(kept) = kept.filter(|kept| !kept.is_null()) {
                        members.insert(name.to_string(), kept.clone());
                    }
                }
            }
        }
        for (name, value) in previous.into_iter().flatten() {
            if !GROUP_MEMBERS.contains(&name.as_str()) {
                members.insert(name.clone(), value.clone());
            }
        }
        Value::Object(members)
    }
}

impl Document {
    /// Every group of the terrain, as saved.
    pub fn groups(&self) -> Vec<GroupData> {
        let Some(Value::Object(saved)) = self.terrain().extra.get("Groups") else {
            return Vec::new();
        };
        let mut list = Vec::new();
        for (key, value) in saved {
            let Ok(id) = key.parse::<i32>() else {
                continue;
            };
            list.push(GroupData::from_json(id, value));
        }
        list
    }

    pub fn group(&self, id: i32) -> Option<GroupData> {
        let saved = self.terrain().extra.get("Groups")?;
        let value = saved.get(id.to_string())?;
        Some(GroupData::from_json(id, value))
    }

    /// The group a node or note belongs to (`GetElementGroup`).
    pub fn group_of(&self, element: i32) -> Option<i32> {
        let groups = self.groups();
        let owner = groups.iter().find(|group| group.children.contains(&element));
        owner.map(|group| group.id)
    }

    /// Writes a group back, made when missing. Not an edit of its own.
    pub(crate) fn store_group(&mut self, group: &GroupData) {
        let extra = &mut self.project.terrain_mut().extra;
        let groups = extra.entry("Groups".to_string()).or_insert_with(|| json!({}));
        if !groups.is_object() {
            *groups = json!({});
        }
        let Value::Object(groups) = groups else {
            return;
        };
        let key = group.id.to_string();
        let previous = groups.get(&key).and_then(Value::as_object).cloned();
        let value = group.to_json(previous.as_ref());
        groups.insert(key, value);
        self.dirty = true;
    }

    fn remove_group_entry(&mut self, id: i32) {
        if let Some(Value::Object(groups)) = self.project.terrain_mut().extra.get_mut("Groups") {
            groups.shift_remove(&id.to_string());
        }
        self.dirty = true;
    }

    /// Changes one group as an edit named `label`.
    fn edit_group(&mut self, id: i32, label: &str, edit: impl FnOnce(&mut GroupData)) {
        let Some(mut group) = self.group(id) else {
            return;
        };
        let before = group.clone();
        edit(&mut group);
        if group == before {
            return;
        }
        self.checkpoint(label);
        self.store_group(&group);
    }

    /// `GraphArea.CreateGroup` / `GraphControl.AddGroup`: a new grey group
    /// on `tab` holding `children`, at `position` and `size`. Returns its
    /// id.
    pub fn add_group(&mut self, children: &[i32], position: [f32; 2], size: [f32; 2], tab: usize) -> i32 {
        self.checkpoint("Group added.");
        let id = self.new_node_id();
        let group = GroupData {
            id,
            name: "Group".to_string(),
            color: "Gray".to_string(),
            icon: String::new(),
            children: children.to_vec(),
            locked: false,
            collapsed: false,
            position,
            size,
            graph_index: tab,
        };
        self.store_group(&group);
        id
    }

    pub fn rename_group(&mut self, id: i32, name: &str) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        self.edit_group(id, "Group renamed", |group| group.name = name);
    }

    /// `MiColorMenuItem_Click`.
    pub fn set_group_color(&mut self, id: i32, color: &str) {
        self.edit_group(id, "Group color", |group| group.color = color.to_string());
    }

    /// `MiIconMenuItem_Click`.
    pub fn set_group_icon(&mut self, id: i32, icon: &str) {
        self.edit_group(id, "Group icon", |group| group.icon = icon.to_string());
    }

    /// `MiLock_Click` and `LockGroup` (F).
    pub fn set_group_locked(&mut self, id: i32, locked: bool) {
        let label = if locked { "Group locked" } else { "Group unlocked" };
        self.edit_group(id, label, |group| group.locked = locked);
    }

    /// `GroupElement_MouseDoubleClick`: a group's header double-clicked
    /// collapses or expands it, unless it is locked.
    pub fn toggle_group_collapsed(&mut self, id: i32) {
        let Some(group) = self.group(id) else {
            return;
        };
        if group.locked {
            return;
        }
        let label = if group.collapsed { "Group expanded" } else { "Group collapsed" };
        self.edit_group(id, label, |group| group.collapsed = !group.collapsed);
    }

    /// `UpdateChildrenPosition`: the group moved to `position` and its
    /// nodes and notes by as much. `checkpoint` is false while a drag is
    /// under way.
    pub fn move_group(&mut self, id: i32, position: [f32; 2], checkpoint: bool) {
        let Some(mut group) = self.group(id) else {
            return;
        };
        if checkpoint {
            let label = format!("{} moved", group.name);
            self.checkpoint(label);
        }
        let delta = [position[0] - group.position[0], position[1] - group.position[1]];
        group.position = position;
        self.store_group(&group);
        let mut moves = Vec::new();
        for child in &group.children {
            if let Some(node) = self.node(*child) {
                let moved = [node.position[0] + delta[0], node.position[1] + delta[1]];
                moves.push((*child, moved));
            }
        }
        self.move_nodes(&moves, false);
        let notes: Vec<(i32, [f32; 2])> = self.notes().iter().filter(|(note, _)| group.children.contains(note)).map(|(note, fields)| (*note, note_position(fields))).collect();
        for (note, start) in notes {
            let moved = [start[0] + delta[0], start[1] + delta[1]];
            self.set_note_vector(note, "Position", moved, false);
        }
    }

    /// The resize thumbs (`ResizeThumb_DragDelta`): a new box for the
    /// group, its children staying where they are.
    pub fn set_group_bounds(&mut self, id: i32, position: [f32; 2], size: [f32; 2]) {
        let Some(mut group) = self.group(id) else {
            return;
        };
        group.position = position;
        group.size = size;
        self.store_group(&group);
    }

    /// `GroupElement.AddChildren`: nodes and notes join the group (part of
    /// whatever edit moved them there).
    pub fn add_to_group(&mut self, id: i32, elements: &[i32]) {
        let Some(mut group) = self.group(id) else {
            return;
        };
        for element in elements {
            group.children.push(*element);
        }
        self.store_group(&group);
    }

    /// `GroupElement.RemoveChildren`.
    pub fn remove_from_group(&mut self, id: i32, elements: &[i32]) {
        let Some(mut group) = self.group(id) else {
            return;
        };
        group.children.retain(|child| !elements.contains(child));
        self.store_group(&group);
    }

    /// `NodeElement_MoveCompleted` / `NoteElement_MoveCompleted` (and
    /// `AddNode`): an element whose corner left its group's box leaves the
    /// group; one in no group joins the first group of its tab whose box
    /// holds its corner.
    pub fn settle_in_groups(&mut self, elements: &[(i32, [f32; 2], usize)]) {
        for (element, corner, tab) in elements {
            match self.group_of(*element) {
                Some(owner) => {
                    let inside = self.group(owner).is_some_and(|group| group.contains(*corner));
                    if !inside {
                        self.remove_from_group(owner, &[*element]);
                    }
                }
                None => {
                    let groups = self.groups();
                    let host = groups.iter().find(|group| group.graph_index == *tab && group.contains(*corner));
                    if let Some(host) = host {
                        let host = host.id;
                        self.add_to_group(host, &[*element]);
                    }
                }
            }
        }
    }

    /// `DeleteGroupElement`: the group goes ("Group deleted."), its nodes
    /// with it when asked (Shift), each deleted as `DeleteNodeElement`
    /// deletes one; otherwise they stay where they are. A locked group
    /// stays.
    pub fn delete_group(&mut self, id: i32, with_children: bool) -> bool {
        let Some(group) = self.group(id) else {
            return false;
        };
        if group.locked {
            return false;
        }
        self.checkpoint("Group deleted.");
        if with_children {
            let nodes: Vec<i32> = group.children.iter().copied().filter(|child| self.node(*child).is_some()).collect();
            for node in nodes {
                self.delete_node_healing(node);
            }
        }
        self.remove_group_entry(id);
        true
    }

    /// The groups of a tab go with it (`DeleteGraphTab`).
    pub(crate) fn remove_tab_groups(&mut self, tab: usize) {
        let doomed: Vec<i32> = self.groups().iter().filter(|group| group.graph_index == tab).map(|group| group.id).collect();
        for id in doomed {
            self.remove_group_entry(id);
        }
    }

    /// Elements that are gone leave their groups.
    pub(crate) fn forget_group_children(&mut self, elements: &[i32]) {
        for group in self.groups() {
            let stale = group.children.iter().any(|child| elements.contains(child));
            if stale {
                self.remove_from_group(group.id, elements);
            }
        }
    }
}

fn note_position(fields: &Map<String, Value>) -> [f32; 2] {
    let position = fields.get("Position");
    let component = |axis: &str| position.and_then(|position| position.get(axis)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
    [component("X"), component("Y")]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_save_their_members_in_order() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let id = document.add_group(&[mountain], [25980.0, 25960.0], [280.0, 270.0], 0);
        let saved = document.terrain().extra["Groups"][id.to_string()].clone();
        let members: Vec<&String> = saved.as_object().unwrap().keys().collect();
        assert_eq!(members, ["Id", "Name", "Color", "Children", "Position", "Size"]);
        assert_eq!(saved["Children"], json!([mountain]));
        assert_eq!(document.group_of(mountain), Some(id));

        document.set_group_locked(id, true);
        assert!(!document.delete_group(id, false));
        document.set_group_locked(id, false);
        document.toggle_group_collapsed(id);
        let group = document.group(id).unwrap();
        assert!(group.collapsed);
        assert_eq!(group.bounds().size(), egui::Vec2::splat(COLLAPSED_SIZE));
        assert!(document.delete_group(id, false));
        assert!(document.group(id).is_none());
        assert!(document.node(mountain).is_some());
    }

    #[test]
    fn moving_a_group_moves_its_children() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let note = document.add_note([26010.0, 26300.0], 0);
        let id = document.add_group(&[mountain, note], [25980.0, 25960.0], [300.0, 400.0], 0);
        document.move_group(id, [25990.0, 25900.0], true);
        assert_eq!(document.node(mountain).unwrap().position, [26010.0, 25940.0]);
        let notes = document.notes();
        let (_, fields) = notes.iter().find(|(found, _)| *found == note).unwrap();
        assert_eq!(note_position(fields), [26020.0, 26240.0]);
    }

    #[test]
    fn elements_settle_in_the_group_under_their_corner() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [25000.0, 25000.0]).unwrap();
        let id = document.add_group(&[], [26000.0, 26000.0], [300.0, 300.0], 0);
        document.settle_in_groups(&[(mountain, [26100.0, 26100.0], 0)]);
        assert_eq!(document.group_of(mountain), Some(id));
        document.settle_in_groups(&[(mountain, [25000.0, 25000.0], 0)]);
        assert_eq!(document.group_of(mountain), None);
    }
}
