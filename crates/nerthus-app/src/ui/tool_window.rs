//! Where the shell's tool windows go: in its body, between the title bar
//! and the status bar, centred when they first open. egui squeezes a window
//! into that body when the body gets smaller than it, and keeps the
//! squeezed size and place; this puts back what it had as the body grows
//! again. [`part`] keeps a part of a window to its place, so that what it
//! holds cannot make the window grow. [`take_escape`] lets Esc close the
//! window on top.

use egui::{Align2, AreaState, Context, Id, Key, LayerId, Layout, Modifiers, Order, Pos2, Rect, Ui, Vec2};

use crate::ui::{alert, status_bar, title_bar};

/// A tool window's margin (Edit Math Node's and the Constructor's 12).
/// egui gives its title bar the same margin, which sets the title bar's
/// height: a window whose content reaches its edges keeps it on the title.
pub const MARGIN: i8 = 12;

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
    window = window.default_size(size);
    // egui keeps a window's place at its pivot and works its edges out
    // from there, so a centred window sized by its bottom right grip moves
    // its top left corner by half the change, and the grip falls behind
    // the pointer. So a window is centred when it opens, then held by its
    // top left corner.
    let shown = AreaState::load(ctx, id).filter(|state| state.size.is_some());
    match shown {
        None => {
            window = window.pivot(Align2::CENTER_CENTER);
            window = window.default_pos(body.center());
        }
        Some(state) => {
            window = window.pivot(Align2::LEFT_TOP);
            if state.pivot != Align2::LEFT_TOP {
                // Not movable for the frame, as below.
                window = window.movable(false);
                window = window.current_pos(state.left_top_pos());
            }
        }
    }
    let key = id.with("squeezed");
    let last = shown.map(|state| state.rect());
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
        window = window.current_pos(wanted.min);
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

/// A part of a window, drawn at `rect` and kept to it: what does not fit
/// is cut off rather than making the window larger.
pub fn part(ui: &mut Ui, id: &str, rect: Rect, layout: Layout) -> Ui {
    let mut part = ui.new_child(egui::UiBuilder::new().id_salt(id).max_rect(rect).layout(layout));
    part.set_clip_rect(rect.intersect(ui.clip_rect()));
    part
}

/// Keeps keyboard traversal and editor widgets behind a dialog on its
/// layer. Windows opened above it (such as Diagnostics from Options)
/// remain usable, as do the dialog's menus and message boxes.
/// Call before drawing the editor: text fields process their events
/// before egui surrenders focus on a blocked layer.
pub fn focus_scope(ctx: &Context, window: Id) {
    let layer = LayerId::new(Order::Middle, window);
    ctx.memory_mut(|memory| memory.set_modal_layer(layer));
    let focused = ctx.memory(|memory| memory.focused());
    let response = focused.and_then(|id| ctx.read_response(id));
    if let Some(response) = response {
        let blocked = ctx.memory(|memory| {
            let below_window = if response.layer_id.order != layer.order {
                response.layer_id.order < layer.order
            } else {
                let focused_position = memory.layer_ids().position(|shown| shown == response.layer_id);
                let window_position = memory.layer_ids().position(|shown| shown == layer);
                match (focused_position, window_position) {
                    (Some(focused), Some(window)) => focused < window,
                    _ => false,
                }
            };
            let below_modal = !memory.is_above_modal_layer(response.layer_id);
            below_window || below_modal
        });
        if blocked {
            ctx.memory_mut(|memory| memory.surrender_focus(response.id));
        }
    }
    ctx.data_mut(|data| data.insert_temp(Id::new("tool-window-focus-scope"), window));
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

/// The window on top: of the windows shown last frame, the one raised
/// last, those in front (About, the Painter, a list dropped down) over the
/// rest. The shell's alert and egui's modals are handled separately.
pub fn on_top(ctx: &Context) -> Option<Id> {
    let alert = alert::area_id();
    let scope = ctx.data(|data| data.get_temp::<Id>(Id::new("tool-window-focus-scope")));
    ctx.memory(|memory| {
        let areas = memory.areas();
        let modal = memory.top_modal_layer();
        let window = |layer: &LayerId| {
            matches!(layer.order, Order::Middle | Order::Foreground)
                && layer.id != alert
                && (Some(*layer) != modal || Some(layer.id) == scope)
                && areas.parent_layer(*layer).is_none()
                && areas.visible_last_frame(layer)
        };
        let shown: Vec<LayerId> = memory.layer_ids().filter(window).collect();
        let top = |order: Order| shown.iter().rfind(|layer| layer.order == order).map(|layer| layer.id);
        top(Order::Foreground).or_else(|| top(Order::Middle))
    })
}

/// Esc closes the window on top (`IsCancel`) when it is one of `windows`,
/// as its close button does: taken before the shell's keys, so that it
/// does not stop a build as well. Returns the window to close. A modal or
/// a list dropped down takes Esc instead, and so does a field in the
/// shell being typed in, which Esc puts back.
pub fn take_escape(ctx: &Context, windows: &[Id]) -> Option<Id> {
    let top = on_top(ctx).filter(|top| windows.contains(top))?;
    let scope = ctx.data(|data| data.get_temp::<Id>(Id::new("tool-window-focus-scope")));
    let modal = ctx.memory(|memory| memory.top_modal_layer().is_some_and(|layer| Some(layer.id) != scope));
    let typing_elsewhere = shell_has_keyboard(ctx);
    if modal || typing_elsewhere || egui::Popup::is_any_open(ctx) {
        return None;
    }
    ctx.input_mut(|input| input.consume_key(Modifiers::NONE, Key::Escape)).then_some(top)
}

/// Asks a window to run its usual close action when it is drawn. Keeping
/// that action in the window preserves Cancel's rollback and cleanup.
pub fn request_close(ctx: &Context, window: Id) {
    ctx.data_mut(|data| data.insert_temp(window.with("close-requested"), true));
}

/// Takes the close request routed to this window before the shell's keys.
pub fn close_requested(ctx: &Context, window: Id) -> bool {
    ctx.data_mut(|data| data.remove_temp::<bool>(window.with("close-requested"))).unwrap_or(false)
}

/// Notes the layer of the widget with the keyboard, once the frame is
/// drawn: egui takes the keyboard back for an Esc before the next frame
/// is drawn.
pub fn note_keyboard(ctx: &Context) {
    let focused = ctx.memory(|memory| memory.focused());
    let layer = focused.and_then(|id| ctx.read_response(id)).map(|response| response.layer_id);
    ctx.data_mut(|data| data.insert_temp(keyboard_id(), layer));
}

/// Whether a field in the shell had the keyboard before egui handled Esc.
pub fn shell_has_keyboard(ctx: &Context) -> bool {
    keyboard_layer(ctx).is_some_and(|layer| layer.order == Order::Background)
}

/// The layer of the widget that had the keyboard last frame.
fn keyboard_layer(ctx: &Context) -> Option<LayerId> {
    ctx.data(|data| data.get_temp::<Option<LayerId>>(keyboard_id())).flatten()
}

fn keyboard_id() -> Id {
    Id::new("tool-window-keyboard")
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

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// Presses at `from`, moves to `to` a step at a time, as a hand does,
    /// and lets go there.
    fn drag(context: &Context, screen: Vec2, size: Vec2, from: Pos2, to: Pos2) {
        frame(context, screen, size, vec![egui::Event::PointerMoved(from)]);
        frame(context, screen, size, vec![button(from, true)]);
        for step in 1..=8 {
            let pos = from + (to - from) * (step as f32 / 8.0);
            frame(context, screen, size, vec![egui::Event::PointerMoved(pos)]);
        }
        frame(context, screen, size, vec![button(to, false)]);
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
    fn sizing_a_window_by_its_corner_keeps_its_top_left_corner() {
        let context = Context::default();
        let screen = Vec2::new(1600.0, 960.0);
        let size = Vec2::new(800.0, 600.0);
        let opened = settle(&context, screen, size);
        // The bottom right grip, dragged up and left.
        let grip = opened.max - Vec2::splat(2.0);
        let to = grip - Vec2::new(200.0, 160.0);
        drag(&context, screen, size, grip, to);
        let smaller = settle(&context, screen, size);
        let wanted = Rect::from_min_max(opened.min, opened.max - Vec2::new(200.0, 160.0));
        assert!(close(smaller, wanted), "{smaller:?} is not {wanted:?}: its top left moved, or its corner left the pointer");
        // Then down and right, to make it larger again.
        let back = to + Vec2::new(124.0, 129.0);
        drag(&context, screen, size, to, back);
        let larger = settle(&context, screen, size);
        let wanted = Rect::from_min_max(opened.min, wanted.max + Vec2::new(124.0, 129.0));
        assert!(close(larger, wanted), "{larger:?} is not {wanted:?}: its top left moved, or its corner left the pointer");
    }

    const SCREEN: Vec2 = Vec2::new(1333.0, 800.0);

    fn escape() -> egui::Event {
        egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }
    }

    /// A frame with `events`: Esc taken for `closable`, as the shell takes
    /// it before it draws, then a field in the shell, then `windows` side
    /// by side, each with a field, then `over` (drawn over them). Returns
    /// the window Esc closed, and whether what was drawn saw the Esc.
    fn escape_frame(context: &Context, windows: &[&str], closable: &[&str], over: impl Fn(&Context), events: Vec<egui::Event>) -> (Option<Id>, bool) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, SCREEN)), events, ..Default::default() };
        let closable: Vec<Id> = closable.iter().map(|name| Id::new(*name)).collect();
        let mut closed = None;
        let mut seen = false;
        let output = context.run_ui(input, |ui| {
            let ctx = ui.ctx().clone();
            closed = take_escape(&ctx, &closable);
            seen = ctx.input(|input| input.key_pressed(Key::Escape));
            let mut text = String::new();
            ui.add(egui::TextEdit::singleline(&mut text).id(Id::new("shell-field")));
            for (index, name) in windows.iter().enumerate() {
                let mut window = egui::Window::new(*name).id(Id::new(*name));
                window = window.default_pos(Pos2::new(100.0 + 400.0 * index as f32, 200.0));
                window.show(&ctx, |ui| {
                    let mut text = String::new();
                    ui.add(egui::TextEdit::singleline(&mut text).id(Id::new(*name).with("field")));
                });
            }
            over(&ctx);
            note_keyboard(&ctx);
        });
        output.drop_without_applying_deltas();
        (closed, seen)
    }

    /// Frames of `windows` with nothing over them until they settle.
    fn show_windows(context: &Context, windows: &[&str]) {
        for _ in 0..3 {
            escape_frame(context, windows, &[], |_| {}, Vec::new());
        }
    }

    /// A click in the middle of `window`, which raises it.
    fn click_in(context: &Context, windows: &[&str], window: &str) {
        let rect = AreaState::load(context, Id::new(window)).expect("the window was shown").rect();
        for event in [egui::Event::PointerMoved(rect.center()), button(rect.center(), true), button(rect.center(), false)] {
            escape_frame(context, windows, &[], |_| {}, vec![event]);
        }
    }

    fn focus(context: &Context, id: Id) {
        context.memory_mut(|memory| memory.request_focus(id));
    }

    #[test]
    fn escape_closes_the_window_on_top() {
        let context = Context::default();
        let windows = ["first", "second"];
        show_windows(&context, &windows);
        // The window opened last is on top.
        let (closed, seen) = escape_frame(&context, &windows, &windows, |_| {}, vec![escape()]);
        assert_eq!(closed, Some(Id::new("second")));
        assert!(!seen, "the shell saw the Esc that closed the window");
        // A click raises the other.
        click_in(&context, &windows, "first");
        let (closed, _) = escape_frame(&context, &windows, &windows, |_| {}, vec![escape()]);
        assert_eq!(closed, Some(Id::new("first")));
    }

    #[test]
    fn escape_is_left_to_a_window_on_top_that_it_does_not_close() {
        let context = Context::default();
        let windows = ["closable", "other"];
        show_windows(&context, &windows);
        let (closed, seen) = escape_frame(&context, &windows, &["closable"], |_| {}, vec![escape()]);
        assert_eq!(closed, None);
        assert!(seen);
    }

    #[test]
    fn a_window_in_front_keeps_escape_from_the_windows_behind() {
        let context = Context::default();
        let windows = ["window"];
        // About is in front of the window, though shown before it.
        let about = |ctx: &Context| {
            egui::Window::new("About").id(Id::new("about")).order(Order::Foreground).show(ctx, |ui| ui.label("About"));
        };
        for _ in 0..3 {
            escape_frame(&context, &[], &[], about, Vec::new());
        }
        for _ in 0..3 {
            escape_frame(&context, &windows, &[], about, Vec::new());
        }
        assert_eq!(on_top(&context), Some(Id::new("about")));
        let (closed, seen) = escape_frame(&context, &windows, &windows, about, vec![escape()]);
        assert_eq!(closed, None);
        assert!(seen);
    }

    #[test]
    fn escape_is_left_to_a_field_typed_in_outside_the_windows() {
        let context = Context::default();
        let windows = ["window"];
        show_windows(&context, &windows);
        focus(&context, Id::new("shell-field"));
        show_windows(&context, &windows);
        let (closed, seen) = escape_frame(&context, &windows, &windows, |_| {}, vec![escape()]);
        assert_eq!(closed, None);
        assert!(seen, "the field did not see the Esc that puts it back");
    }

    #[test]
    fn escape_closes_a_window_while_its_own_field_is_typed_in() {
        let context = Context::default();
        let windows = ["window"];
        show_windows(&context, &windows);
        focus(&context, Id::new("window").with("field"));
        show_windows(&context, &windows);
        let (closed, _) = escape_frame(&context, &windows, &windows, |_| {}, vec![escape()]);
        assert_eq!(closed, Some(Id::new("window")));
    }

    #[test]
    fn escape_is_left_to_a_modal_over_the_window() {
        let context = Context::default();
        let windows = ["window"];
        let modal = |ctx: &Context| {
            egui::Modal::new(Id::new("modal")).show(ctx, |ui| ui.label("Modal"));
        };
        for _ in 0..3 {
            escape_frame(&context, &windows, &[], modal, Vec::new());
        }
        let (closed, seen) = escape_frame(&context, &windows, &windows, modal, vec![escape()]);
        assert_eq!(closed, None);
        assert!(seen);
    }

    #[test]
    fn the_alert_is_not_the_window_on_top() {
        let context = Context::default();
        let windows = ["window"];
        show_windows(&context, &windows);
        // The alert shows (and so is raised) after the window.
        let alert = |ctx: &Context| {
            egui::Area::new(alert::area_id()).order(Order::Middle).show(ctx, |ui| ui.label("Alert"));
        };
        for _ in 0..3 {
            escape_frame(&context, &windows, &[], alert, Vec::new());
        }
        assert_eq!(on_top(&context), Some(Id::new("window")));
        let (closed, _) = escape_frame(&context, &windows, &windows, alert, vec![escape()]);
        assert_eq!(closed, Some(Id::new("window")));
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

#[cfg(test)]
mod focus_tests {
    use super::*;
    use egui::{Event, PointerButton, RawInput};

    #[derive(Default)]
    struct FocusFrame {
        shell_ids: Vec<Id>,
        shell_text: String,
        options_ids: Vec<Id>,
        diagnostics_id: Option<Id>,
        modal_ids: Vec<Id>,
        activated: Vec<&'static str>,
    }

    /// Match the editor's drawing order: focus preflight, shell widgets,
    /// a possible foreground question, Options, and then Diagnostics.
    fn frame(context: &Context, diagnostics: bool, modal: bool, events: Vec<Event>) -> FocusFrame {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let input = RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut result = FocusFrame::default();
        let output = context.run_ui(input, |ui| {
            let context = ui.ctx().clone();
            focus_scope(&context, Id::new("focus-options"));
            let field = egui::TextEdit::singleline(&mut result.shell_text).id(Id::new("focus-shell-field"));
            let response = ui.add(field);
            result.shell_ids.push(response.id);
            for label in ["Shell first", "Shell last"] {
                let response = ui.button(label);
                result.shell_ids.push(response.id);
                if response.clicked() {
                    result.activated.push(label);
                }
            }

            if modal {
                egui::Modal::new(Id::new("focus-modal")).show(&context, |ui| {
                    for label in ["Modal yes", "Modal no"] {
                        let response = ui.button(label);
                        result.modal_ids.push(response.id);
                        if response.clicked() {
                            result.activated.push(label);
                        }
                    }
                });
            }

            focus_scope(&context, Id::new("focus-options"));
            let mut options = egui::Window::new("Focus Options");
            options = options.id(Id::new("focus-options"));
            options = options.collapsible(false);
            options = options.default_pos(Pos2::new(180.0, 100.0));
            options.show(&context, |ui| {
                for label in ["Options first", "Accept Changes", "Cancel"] {
                    let response = ui.button(label);
                    result.options_ids.push(response.id);
                    if response.clicked() {
                        result.activated.push(label);
                    }
                }
            });

            if diagnostics {
                let mut diagnostics = egui::Window::new("Focus Diagnostics");
                diagnostics = diagnostics.id(Id::new("focus-diagnostics"));
                diagnostics = diagnostics.collapsible(false);
                diagnostics = diagnostics.default_pos(Pos2::new(600.0, 100.0));
                diagnostics.show(&context, |ui| {
                    let response = ui.button("Run Diagnostics");
                    result.diagnostics_id = Some(response.id);
                    if response.clicked() {
                        result.activated.push("Run Diagnostics");
                    }
                });
            }
        });
        output.drop_without_applying_deltas();
        result
    }

    fn settle(context: &Context, diagnostics: bool, modal: bool) -> FocusFrame {
        for _ in 0..3 {
            frame(context, diagnostics, modal, Vec::new());
        }
        frame(context, diagnostics, modal, Vec::new())
    }

    fn key(key: Key, modifiers: Modifiers) -> Vec<Event> {
        let pressed = Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers };
        let released = Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers };
        vec![pressed, released]
    }

    fn assert_focused_on(context: &Context, layer: &str) {
        let focused = context.memory(|memory| memory.focused()).expect("a scoped widget has focus");
        let response = context.read_response(focused).expect("the focused widget was drawn");
        assert_eq!(response.layer_id.id, Id::new(layer));
    }

    #[test]
    fn tab_and_shift_tab_wrap_without_entering_editor_widgets() {
        let context = Context::default();
        let shown = settle(&context, false, false);
        context.memory_mut(|memory| memory.request_focus(shown.options_ids[0]));
        let mut visited = Vec::new();

        for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
            for _ in 0..9 {
                frame(&context, false, false, key(Key::Tab, modifiers));
                let focused = context.memory(|memory| memory.focused());
                assert!(!shown.shell_ids.iter().any(|id| Some(*id) == focused));
                // egui completes wrapping to the first or last widget at
                // the start of the following pass.
                frame(&context, false, false, Vec::new());
                assert_focused_on(&context, "focus-options");
                visited.push(context.memory(|memory| memory.focused()).unwrap());
            }
        }

        for id in shown.options_ids {
            assert!(visited.contains(&id), "Options widget {id:?} was skipped");
        }
    }

    #[test]
    fn space_enter_and_text_cannot_activate_editor_widgets_behind_the_scope() {
        let context = Context::default();
        let shown = settle(&context, false, false);
        for id in &shown.shell_ids {
            for activation in [Key::Space, Key::Enter] {
                context.memory_mut(|memory| memory.request_focus(*id));
                let result = frame(&context, false, false, key(activation, Modifiers::NONE));
                assert!(result.activated.is_empty(), "a shell widget activated: {:?}", result.activated);
            }
        }
        context.memory_mut(|memory| memory.request_focus(shown.shell_ids[0]));
        let result = frame(&context, false, false, vec![Event::Text("shell must stay unchanged".to_string())]);
        assert!(result.shell_text.is_empty());

        for activation in [Key::Space, Key::Enter] {
            context.memory_mut(|memory| memory.request_focus(shown.options_ids[0]));
            let result = frame(&context, false, false, key(activation, Modifiers::NONE));
            assert_eq!(result.activated, vec!["Options first"]);
        }
    }

    #[test]
    fn opening_the_scope_clears_an_editor_text_fields_focus_before_typing() {
        let context = Context::default();
        let shell_id = Id::new("focus-shell-field");
        let mut shell_text = String::new();
        let output = context.run_ui(RawInput::default(), |ui| {
            let field = egui::TextEdit::singleline(&mut shell_text).id(shell_id);
            ui.add(field).request_focus();
        });
        output.drop_without_applying_deltas();
        assert_eq!(context.memory(|memory| memory.focused()), Some(shell_id));

        let result = frame(&context, false, false, vec![Event::Text("hidden edit".to_string())]);
        assert!(result.shell_text.is_empty());
        assert_ne!(context.memory(|memory| memory.focused()), Some(shell_id));
    }

    #[test]
    fn a_higher_diagnostics_window_keeps_keyboard_and_pointer_input() {
        let context = Context::default();
        let shown = settle(&context, true, false);
        let diagnostics_id = shown.diagnostics_id.unwrap();
        assert_eq!(on_top(&context), Some(Id::new("focus-diagnostics")));

        context.memory_mut(|memory| memory.request_focus(diagnostics_id));
        let result = frame(&context, true, false, key(Key::Enter, Modifiers::NONE));
        assert_eq!(result.activated, vec!["Run Diagnostics"]);
        assert_focused_on(&context, "focus-diagnostics");

        let position = context.read_response(diagnostics_id).unwrap().rect.center();
        frame(&context, true, false, vec![Event::PointerMoved(position)]);
        let pressed = Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE };
        frame(&context, true, false, vec![pressed]);
        let released = Event::PointerButton { pos: position, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE };
        let result = frame(&context, true, false, vec![released]);
        assert_eq!(result.activated, vec!["Run Diagnostics"]);
    }

    #[test]
    fn a_foreground_modal_drawn_before_options_keeps_its_keyboard_scope() {
        let context = Context::default();
        let shown = settle(&context, true, true);
        let modal = context.memory(|memory| memory.top_modal_layer()).unwrap();
        assert_eq!(modal.id, Id::new("focus-modal"));

        context.memory_mut(|memory| memory.request_focus(shown.modal_ids[0]));
        let result = frame(&context, true, true, key(Key::Enter, Modifiers::NONE));
        assert_eq!(result.activated, vec!["Modal yes"]);
        for modifiers in [Modifiers::NONE, Modifiers::SHIFT] {
            for _ in 0..4 {
                frame(&context, true, true, key(Key::Tab, modifiers));
                frame(&context, true, true, Vec::new());
                assert_focused_on(&context, "focus-modal");
            }
        }

        for id in [shown.options_ids[0], shown.diagnostics_id.unwrap()] {
            context.memory_mut(|memory| memory.request_focus(id));
            let result = frame(&context, true, true, key(Key::Space, Modifiers::NONE));
            assert!(result.activated.is_empty(), "a covered window activated: {:?}", result.activated);
        }
    }
}
