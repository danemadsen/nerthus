//! Values typed into a slider are worked out as formulas: the project's
//! float, integer and boolean variables are constants, with `Resolution`,
//! `PX` and the `m()`, `cm()` and `km()` heights besides. Every slider
//! reads them from the frame's context, so the property area, the Data
//! View, the lighting panel and the Constructor all take formulas.

use nerthus_engine::automation::{Automation, ExpressionContext};
use nerthus_engine::Resolution;

use crate::editor::Editor;

/// What typed values are worked out against this frame.
#[derive(Clone)]
struct Formulas {
    /// The project's variables (bindings and expressions play no part).
    automation: std::sync::Arc<Automation>,
    context: ExpressionContext,
}

fn formulas_id() -> egui::Id {
    egui::Id::new("slider-formulas")
}

/// Makes the open project's variables, the preview resolution and the
/// terrain's height what typed values are worked out against.
pub fn publish(ctx: &egui::Context, editor: &Editor) {
    let variables = editor.document.automation().variables.clone();
    let automation = Automation { variables, ..Default::default() };
    let context = ExpressionContext {
        resolution: Resolution::square(editor.preview_resolution),
        terrain_height: editor.document.terrain().height,
    };
    let formulas = Formulas { automation: std::sync::Arc::new(automation), context };
    ctx.data_mut(|data| data.insert_temp(formulas_id(), formulas));
}

/// The typed text's value, or `None` when it does not work out. Before
/// a project publishes its variables, plain numbers and the built-in
/// functions still work.
pub fn evaluate(ctx: &egui::Context, text: &str) -> Option<f32> {
    let published: Option<Formulas> = ctx.data(|data| data.get_temp(formulas_id()));
    match published {
        Some(formulas) => formulas.automation.calculate_number(text, &formulas.context),
        None => {
            let context = ExpressionContext {
                resolution: Resolution::square(1024),
                terrain_height: 1.0,
            };
            Automation::default().calculate_number(text, &context)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_text_is_worked_out_as_a_formula() {
        let ctx = egui::Context::default();
        assert_eq!(evaluate(&ctx, "0.25"), Some(0.25));
        assert_eq!(evaluate(&ctx, "1/4 + 0.5"), Some(0.75));
        assert_eq!(evaluate(&ctx, "max(2, 3)"), Some(3.0));
        assert_eq!(evaluate(&ctx, "two"), None);
        assert_eq!(evaluate(&ctx, ""), None);
    }

    #[test]
    fn published_variables_resolution_and_height_count() {
        let ctx = egui::Context::default();
        let mut editor = Editor::new(crate::document::Document::new());
        editor.preview_resolution = 512;
        publish(&ctx, &editor);
        assert_eq!(evaluate(&ctx, "Resolution"), Some(512.0));
        assert_eq!(evaluate(&ctx, "PX * 512"), Some(1.0));
        let height = editor.document.terrain().height;
        let metres = evaluate(&ctx, "m(100)").unwrap();
        assert!((metres - 100.0 / height).abs() < 1e-6, "{metres}");
    }
}
