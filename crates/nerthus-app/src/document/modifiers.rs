//! Modifier edits from the modifier list: a modifier's close button
//! (`EditorBase.Adorner_CloseButtonClick`) and a modifier dragged to
//! another place (`EditorBase.Drop`). Places count the modifiers in
//! `Order`, as the list shows them.

use nerthus_engine::editing::modifier_order;
use nerthus_engine::ModifierData;
use serde_json::Value;

use super::Document;

/// The `Order` Min and Max are made with: they stay last, cannot be
/// dragged and keep it when the others are numbered again.
pub const PINNED_ORDER: i64 = 66;

impl Document {
    /// The node's modifiers by their place in the list: indices into
    /// `NodeData::modifiers`, sorted by `Order` (ties keep their order).
    pub fn modifier_places(&self, node: i32) -> Vec<usize> {
        let Some(data) = self.node(node) else {
            return Vec::new();
        };
        let mut places: Vec<usize> = (0..data.modifiers.len()).collect();
        places.sort_by_key(|index| modifier_order(&data.modifiers[*index]));
        places
    }

    /// The close button: the modifier at `place` goes, whichever way it
    /// was added ("X modifier removed"; the close button itself leaves no
    /// undo step, which is not kept).
    pub fn remove_modifier_at(&mut self, node: i32, place: usize) -> bool {
        let places = self.modifier_places(node);
        let Some(index) = places.get(place).copied() else {
            return false;
        };
        let name = self.node(node).map(|data| data.name.clone()).unwrap_or_default();
        self.checkpoint(format!("{name} modifier removed"));
        if let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) {
            target.modifiers.remove(index);
        }
        self.mark_stale(node);
        true
    }

    /// A drop: the modifier at `from` takes the place of the one at `to`
    /// (taken out, then put back at that place), and every modifier but
    /// Min and Max is numbered again from 0 in the new order ("X modifier
    /// order changed"). Min and Max cannot be moved.
    pub fn move_modifier(&mut self, node: i32, from: usize, to: usize) -> bool {
        let mut places = self.modifier_places(node);
        let count = places.len();
        if from >= count || to >= count || from == to {
            return false;
        }
        let Some(data) = self.node(node) else {
            return false;
        };
        if modifier_order(&data.modifiers[places[from]]) == PINNED_ORDER {
            return false;
        }
        let name = data.name.clone();
        self.checkpoint(format!("{name} modifier order changed"));
        let moved = places.remove(from);
        places.insert(to, moved);
        let Some(target) = self.project.terrain_mut().nodes.get_mut(&node) else {
            return false;
        };
        let mut slots: Vec<Option<ModifierData>> = std::mem::take(&mut target.modifiers).into_iter().map(Some).collect();
        let mut next_order = 0i64;
        for index in places {
            let Some(mut modifier) = slots[index].take() else {
                continue;
            };
            if modifier_order(&modifier) != PINNED_ORDER {
                set_order(&mut modifier, next_order);
                next_order += 1;
            }
            target.modifiers.push(modifier);
        }
        self.mark_stale(node);
        true
    }
}

/// `Modifier.Order` as a project saves it: left out at 0, otherwise the
/// last of the modifier's members (it is `Modifier`'s last).
fn set_order(modifier: &mut ModifierData, order: i64) {
    if order == 0 {
        modifier.values.shift_remove("Order");
        return;
    }
    match modifier.values.get_mut("Order") {
        Some(saved) => *saved = Value::from(order),
        None => {
            modifier.values.insert("Order".to_string(), Value::from(order));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nerthus_engine::editing::{modifier_name, new_modifier};

    /// A Perlin node with these modifiers added from the toolbar, in turn.
    fn document_with(modifiers: &[&str]) -> (Document, i32) {
        let mut document = Document::new();
        let node = document.add_node("Perlin", [0.0, 0.0]).unwrap();
        let target = document.project.terrain_mut().nodes.get_mut(&node).unwrap();
        for (order, name) in modifiers.iter().enumerate() {
            target.modifiers.push(new_modifier(name, order).unwrap());
        }
        (document, node)
    }

    fn names(document: &Document, node: i32) -> Vec<String> {
        let data = document.node(node).unwrap();
        document.modifier_places(node).into_iter().map(|index| modifier_name(&data.modifiers[index]).to_string()).collect()
    }

    fn orders(document: &Document, node: i32) -> Vec<Option<i64>> {
        let data = document.node(node).unwrap();
        data.modifiers.iter().map(|modifier| modifier.values.get("Order").and_then(Value::as_i64)).collect()
    }

    #[test]
    fn a_dropped_modifier_takes_the_target_place() {
        let (mut document, node) = document_with(&["Blur", "Clamp", "Invert"]);
        // Down onto the last: it ends after it.
        assert!(document.move_modifier(node, 0, 2));
        assert_eq!(names(&document, node), ["Clamp", "Invert", "Blur"]);
        // Saved in the new order, numbered from 0 (0 left out).
        assert_eq!(orders(&document, node), [None, Some(1), Some(2)]);
        assert_eq!(document.undo_labels()[0], format!("{} modifier order changed", document.node(node).unwrap().name));
        // Up onto the first: it ends before it.
        assert!(document.move_modifier(node, 2, 0));
        assert_eq!(names(&document, node), ["Blur", "Clamp", "Invert"]);
        assert!(!document.move_modifier(node, 1, 1));
        assert!(!document.move_modifier(node, 0, 3));
    }

    #[test]
    fn min_and_max_stay_put() {
        let (mut document, node) = document_with(&["Blur", "Clamp"]);
        let target = document.project.terrain_mut().nodes.get_mut(&node).unwrap();
        let mut max = new_modifier("Max", 0).unwrap();
        max.values.insert("Order".to_string(), Value::from(PINNED_ORDER));
        target.modifiers.insert(0, max);
        assert_eq!(names(&document, node), ["Blur", "Clamp", "Max"]);
        assert!(!document.move_modifier(node, 2, 0));
        assert!(document.move_modifier(node, 1, 0));
        assert_eq!(names(&document, node), ["Clamp", "Blur", "Max"]);
        let max_order = document.node(node).unwrap().modifiers.iter().find(|modifier| modifier.type_name == "Max").and_then(|modifier| modifier.values.get("Order")).and_then(Value::as_i64);
        assert_eq!(max_order, Some(PINNED_ORDER));
    }

    #[test]
    fn the_close_button_removes_that_modifier() {
        let (mut document, node) = document_with(&["Blur", "Clamp", "Invert"]);
        assert!(document.remove_modifier_at(node, 1));
        assert_eq!(names(&document, node), ["Blur", "Invert"]);
        assert!(document.undo_labels()[0].ends_with("modifier removed"));
        assert!(!document.remove_modifier_at(node, 5));
        document.undo();
        assert_eq!(names(&document, node), ["Blur", "Clamp", "Invert"]);
    }
}
