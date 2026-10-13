//! Note edits from the graph: a note double-clicked between its icon and
//! its text (`NoteElement_MouseDoubleClick`), and its colour from the
//! note's menu (`MiColorMenuItem_Click`).

use serde_json::{json, Value};

use super::Document;

impl Document {
    /// `ChangeNoteType`: an `Icon` note shows its text, a text note becomes
    /// an icon. There is no undo step for it.
    pub fn toggle_note_type(&mut self, id: i32) {
        let Some(note) = self.note_mut(id) else {
            return;
        };
        // `Type` missing from the file is `Icon`, `NoteType`'s first member.
        let is_icon = note.get("Type").and_then(Value::as_str) != Some("OnlyText");
        let next = if is_icon { "OnlyText" } else { "Icon" };
        note.insert("Type".to_string(), json!(next));
        self.dirty = true;
    }

    /// The note's swatch.
    pub fn set_note_color(&mut self, id: i32, color: &str) {
        let unchanged = self.note_mut(id).is_none_or(|note| note.get("Color").and_then(Value::as_str) == Some(color));
        if unchanged {
            return;
        }
        self.checkpoint("Note color");
        if let Some(note) = self.note_mut(id) {
            note.insert("Color".to_string(), json!(color));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_flip_between_icon_and_text() {
        let mut document = Document::new();
        let note = document.add_note([0.0, 0.0], 0);
        document.toggle_note_type(note);
        let kind = |document: &Document| document.notes()[0].1.get("Type").and_then(Value::as_str).map(str::to_string);
        assert_eq!(kind(&document).as_deref(), Some("Icon"));
        document.toggle_note_type(note);
        assert_eq!(kind(&document).as_deref(), Some("OnlyText"));
        document.set_note_color(note, "Teal");
        assert_eq!(document.notes()[0].1.get("Color").and_then(Value::as_str), Some("Teal"));
        assert_eq!(document.undo_labels()[0], "Note color");
    }
}
