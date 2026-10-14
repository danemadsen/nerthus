//! Where the shell's tool windows go: in its body, between the title bar
//! and the status bar, centred when they first open. egui squeezes a window
//! into that body when the body gets smaller than it, and keeps the
//! squeezed size and place; this puts back what it had as the body grows
//! again.

use egui::{AreaState, Context, Id, Pos2, Rect, Vec2};

use crate::ui::{status_bar, title_bar};

/// A window the body got too small for.
#[derive(Clone, Copy, Debug)]
struct Squeezed {
    /// Its place and size before it was squeezed.
    before: Rect,
    /// Whether it opened squeezed, so that it goes to the body's middle.
    centred: bool,
    /// The body, and the window's rect, as it was last shown.
    body: Rect,
    rect: Rect,
}

impl Squeezed {
    /// Where it goes in this body, given the room.
    fn wanted(&self, body: Rect) -> Rect {
        if self.centred { Rect::from_center_size(body.center(), self.before.size()) } else { self.before }
    }
}

/// The shell's body: the screen less the title bar and the status bar.
pub fn body(ctx: &Context) -> Rect {
    let full = ctx.content_rect();
    let top = full.top() + title_bar::HEIGHT;
    let bottom = (full.bottom() - status_bar::HEIGHT).max(top);
    Rect::from_min_max(Pos2::new(full.left(), top), Pos2::new(full.right(), bottom))
}

/// Keeps `window` (shown as `id`, `size` by default) in the body, opens it
/// in the body's middle, and gives it back the size and place it had
/// before the body squeezed it once there is room again. Moving or sizing
/// it by hand while it is squeezed keeps it where it is put.
pub fn place<'open>(ctx: &Context, window: egui::Window<'open>, id: Id, size: Vec2) -> egui::Window<'open> {
    let body = body(ctx);
    let mut window = window.id(id);
    window = window.constrain_to(body);
    window = window.pivot(egui::Align2::CENTER_CENTER);
    window = window.default_pos(body.center());
    window = window.default_size(size);
    let key = id.with("squeezed");
    let last = AreaState::load(ctx, id).filter(|state| state.size.is_some()).map(|state| state.rect());
    let mut squeezed = ctx.data(|data| data.get_temp::<Squeezed>(key));
    if let (Some(kept), Some(rect)) = (squeezed, last)
        && kept.body == body
        && kept.rect != rect
        && dragging(ctx, id)
    {
        squeezed = None;
    }
    // Opening, or shown again, too big for the body: what to grow back to.
    let squeezed = squeezed.or_else(|| {
        let before = last.unwrap_or_else(|| Rect::from_center_size(body.center(), size));
        (!fits(before, body)).then_some(Squeezed { before, centred: last.is_none(), body: Rect::NOTHING, rect: before })
    });
    let Some(mut kept) = squeezed else {
        ctx.data_mut(|data| data.remove::<Squeezed>(key));
        return window;
    };
    // As the body changes, as much of what it had as fits.
    if let Some(last) = last
        && kept.body != body
    {
        let full = kept.wanted(body);
        let wanted = within(Rect::from_center_size(full.center(), full.size().min(body.size())), body);
        // Not movable for the frame: egui puts a window dragged by its
        // title bar back where it was, whatever its `current_pos`.
        window = window.movable(false);
        window = window.current_pos(wanted.center());
        if (last.size() - wanted.size()).abs().max_elem() > 0.5 {
            // For a frame, which leaves egui's resize state at this size.
            window = window.fixed_size(wanted.size());
        }
    }
    if fits(kept.wanted(body), body) {
        ctx.data_mut(|data| data.remove::<Squeezed>(key));
    } else {
        kept.body = body;
        kept.rect = last.unwrap_or(kept.rect);
        ctx.data_mut(|data| data.insert_temp(key, kept));
    }
    window
}

/// Whether `rect` is inside `body` (give or take rounding).
fn fits(rect: Rect, body: Rect) -> bool {
    body.expand(0.5).contains_rect(rect)
}

/// `rect` moved as little as it takes to be inside `body`.
fn within(rect: Rect, body: Rect) -> Rect {
    let most = (body.max - rect.size()).max(body.min);
    Rect::from_min_size(rect.min.clamp(body.min, most), rect.size())
}

/// Whether the window is being moved or sized by hand: what the pointer
/// drags (or just let go of) is on its layer.
fn dragging(ctx: &Context, id: Id) -> bool {
    let dragged = ctx.dragged_id().or_else(|| ctx.drag_stopped_id());
    dragged.and_then(|dragged| ctx.read_response(dragged)).is_some_and(|response| response.layer_id.id == id)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "tool-window-test";

    /// A frame at this screen size (in points) with these events, showing a
    /// resizable window that fills what it is given. Returns its rect.
    fn frame(context: &Context, screen: Vec2, size: Vec2, events: Vec<egui::Event>) -> Rect {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), events, ..Default::default() };
        let mut shown = Rect::NOTHING;
        let output = context.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            let mut window = egui::Window::new("Tool");
            window = window.collapsible(false);
            window = window.resizable(true);
            window = place(&ctx, window, Id::new(ID), size);
            let response = window.show(&ctx, |ui| {
                ui.allocate_space(ui.available_size());
            });
            shown = response.map(|response| response.response.rect).unwrap_or(Rect::NOTHING);
        });
        output.drop_without_applying_deltas();
        shown
    }

    /// Frames enough for egui's first-frame sizing and our changes to settle.
    fn settle(context: &Context, screen: Vec2, size: Vec2) -> Rect {
        for _ in 0..4 {
            frame(context, screen, size, Vec::new());
        }
        frame(context, screen, size, Vec::new())
    }

    fn body_of(screen: Vec2) -> Rect {
        Rect::from_min_max(Pos2::new(0.0, title_bar::HEIGHT), Pos2::new(screen.x, screen.y - status_bar::HEIGHT))
    }

    fn close(a: Rect, b: Rect) -> bool {
        (a.min - b.min).abs().max_elem() <= 1.0 && (a.max - b.max).abs().max_elem() <= 1.0
    }

    #[test]
    fn windows_open_in_the_middle_of_the_body() {
        let context = Context::default();
        let screen = Vec2::new(1333.0, 800.0);
        let size = Vec2::new(400.0, 300.0);
        let rect = settle(&context, screen, size);
        let body = body_of(screen);
        assert!(body.contains_rect(rect), "{rect:?} is outside {body:?}");
        assert!((rect.center() - body.center()).length() <= 1.0, "{rect:?} is not centred in {body:?}");
    }

    #[test]
    fn windows_too_big_for_the_body_stay_below_the_title_bar() {
        let context = Context::default();
        let screen = Vec2::new(800.0, 600.0);
        let rect = settle(&context, screen, Vec2::new(1280.0, 720.0));
        let body = body_of(screen);
        assert!(rect.top() >= body.top() - 0.5, "{rect:?} covers the title bar");
        assert!(rect.bottom() <= body.bottom() + 0.5, "{rect:?} covers the status bar");
    }

    #[test]
    fn squeezed_windows_grow_back() {
        let context = Context::default();
        let large = Vec2::new(1333.0, 800.0);
        let size = Vec2::new(920.0, 640.0);
        let before = settle(&context, large, size);
        // Squeezed into the small body, then given the room back a step at
        // a time, as a window being dragged larger is.
        let small = Vec2::new(533.0, 400.0);
        let squeezed = settle(&context, small, size);
        assert!(body_of(small).contains_rect(squeezed), "{squeezed:?} is outside the small body");
        for step in 1..=8 {
            let screen = small + (large - small) * (step as f32 / 8.0);
            frame(&context, screen, size, Vec::new());
        }
        let after = settle(&context, large, size);
        assert!(close(after, before), "{after:?} is not back at {before:?}");
    }

    #[test]
    fn windows_too_big_at_first_grow_to_their_size() {
        let context = Context::default();
        let size = Vec2::new(1080.0, 760.0);
        let small = Vec2::new(1024.0, 768.0);
        settle(&context, small, size);
        let large = Vec2::new(1600.0, 1000.0);
        let rect = settle(&context, large, size);
        assert!(close(rect, Rect::from_center_size(body_of(large).center(), size)), "{rect:?} did not grow to {size:?}");
    }

    #[test]
    fn windows_go_back_as_far_as_there_is_room() {
        let context = Context::default();
        let large = Vec2::new(1333.0, 800.0);
        let size = Vec2::new(920.0, 640.0);
        settle(&context, large, size);
        settle(&context, Vec2::new(533.0, 400.0), size);
        // Room for the size but not the place: the size, moved inside.
        let middle = Vec2::new(1000.0, 720.0);
        let rect = settle(&context, middle, size);
        assert!((rect.size() - size).abs().max_elem() <= 1.0, "{rect:?} is not {size:?}");
        assert!(body_of(middle).expand(0.5).contains_rect(rect), "{rect:?} is outside the body");
    }

    #[test]
    fn a_window_moved_while_squeezed_stays_put() {
        let context = Context::default();
        let large = Vec2::new(1333.0, 800.0);
        let size = Vec2::new(300.0, 200.0);
        settle(&context, large, size);
        // Not squeezed in size, only pushed in from the right.
        let small = Vec2::new(700.0, 600.0);
        let pushed = settle(&context, small, size);
        // Dragged by its title bar, left and down.
        let from = Pos2::new(pushed.center().x, pushed.top() + 8.0);
        let to = from + Vec2::new(-40.0, 50.0);
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&context, small, size, vec![egui::Event::PointerMoved(from)]);
        frame(&context, small, size, vec![button(from, true)]);
        frame(&context, small, size, vec![egui::Event::PointerMoved(from + Vec2::new(-20.0, 25.0))]);
        frame(&context, small, size, vec![egui::Event::PointerMoved(to)]);
        frame(&context, small, size, vec![button(to, false)]);
        let moved = settle(&context, small, size);
        assert!((moved.min - pushed.min).length() > 10.0, "{moved:?} was not dragged from {pushed:?}");
        let after = settle(&context, large, size);
        assert!(close(after, moved), "{after:?} left where it was put, {moved:?}");
    }

    #[test]
    fn within_moves_rects_inside() {
        let body = Rect::from_min_max(Pos2::new(0.0, 36.0), Pos2::new(500.0, 400.0));
        let rect = Rect::from_min_size(Pos2::new(-50.0, 0.0), Vec2::new(100.0, 100.0));
        assert_eq!(within(rect, body), Rect::from_min_size(Pos2::new(0.0, 36.0), Vec2::new(100.0, 100.0)));
        let rect = Rect::from_min_size(Pos2::new(450.0, 380.0), Vec2::new(100.0, 100.0));
        assert_eq!(within(rect, body), Rect::from_min_size(Pos2::new(400.0, 300.0), Vec2::new(100.0, 100.0)));
    }
}
