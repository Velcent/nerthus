use super::*;

struct TestCanvas {
    context: egui::Context,
    editor: Editor,
    state: RegionsState,
    canvas: Rect,
    names: [String; 2],
    time: f64,
}

impl TestCanvas {
    fn new() -> Self {
        let context = crate::ui::widgets::fit_checks::context();
        let mut document = crate::document::Document::new();
        let lower = document.add_region([0.25, 0.25]);
        let upper = document.add_region([0.30, 0.30]);
        let editor = Editor::new(document);
        let mut view = Self {
            context,
            editor,
            state: RegionsState::default(),
            canvas: Rect::NOTHING,
            names: [lower, upper],
            time: 0.0,
        };
        view.frame(Vec::new(), egui::Modifiers::NONE);
        view.frame(Vec::new(), egui::Modifiers::NONE);
        view
    }

    fn frame(&mut self, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<(String, Rect)> {
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::splat(600.0));
        let mut held_then_events = vec![egui::Event::ModifiersChanged(modifiers)];
        held_then_events.extend(events);
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(self.time),
            events: held_then_events,
            ..Default::default()
        };
        let mut commands = Vec::new();
        let mut output = self.context.run_ui(input, |ui| {
            let side = ui.available_width().min(ui.available_height().max(320.0)).max(160.0);
            let outer = Rect::from_min_size(ui.next_widget_position(), Vec2::splat(side));
            self.canvas = outer.shrink(1.5);
            canvas(ui, &mut self.editor, &mut self.state, &mut commands);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let mut words = Vec::new();
        for clipped in shapes {
            if let egui::Shape::Text(text) = clipped.shape {
                words.push((text.galley.text().to_string(), text.visual_bounding_rect()));
            }
        }
        words
    }

    fn overlap(&self) -> Pos2 {
        self.canvas.min + Vec2::splat(self.canvas.width() * 0.4)
    }

    fn button(&mut self, point: Pos2, pressed: bool, modifiers: egui::Modifiers) {
        let button = egui::Event::PointerButton {
            pos: point,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        let events = vec![egui::Event::PointerMoved(point), button];
        self.frame(events, modifiers);
    }

    fn region(&self, index: usize) -> Region {
        let name = &self.names[index];
        self.editor.document.regions().into_iter().find(|region| &region.name == name).unwrap()
    }
}

#[test]
fn shift_click_opens_the_overlap_chooser_without_a_drag() {
    let mut view = TestCanvas::new();
    let point = view.overlap();
    let before = view.editor.document.regions();
    view.button(point, true, egui::Modifiers::SHIFT);
    assert_eq!(view.state.choices, vec![view.names[1].clone(), view.names[0].clone()]);
    assert_eq!(view.state.choices_at, point);
    assert!(view.state.moving.is_none());
    view.button(point, false, egui::Modifiers::SHIFT);
    view.frame(Vec::new(), egui::Modifiers::NONE);
    assert_eq!(view.state.choices.len(), 2, "the opening click must leave the chooser open");
    assert_eq!(view.editor.document.regions(), before);
}

#[test]
fn alt_click_focuses_the_region_below_and_clears_the_pending_gesture() {
    let mut view = TestCanvas::new();
    let point = view.overlap();
    view.button(point, true, egui::Modifiers::ALT);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    view.button(point, false, egui::Modifiers::ALT);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    assert!(view.state.moving.is_none());
    assert!(view.state.resizing.is_none());

    view.button(point, true, egui::Modifiers::NONE);
    view.button(point, false, egui::Modifiers::NONE);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[1]));
}

#[test]
fn a_chosen_region_moves_on_the_next_press_and_stays_selected_through_the_drag() {
    let mut view = TestCanvas::new();
    let point = view.overlap();
    let lower_before = view.region(0);
    let upper_before = view.region(1);
    view.button(point, true, egui::Modifiers::SHIFT);
    view.button(point, false, egui::Modifiers::SHIFT);
    let words = view.frame(Vec::new(), egui::Modifiers::NONE);
    let menu_rect = egui::AreaState::load(&view.context, Id::new("region-choices")).unwrap().rect();
    let lower_row = words.iter().find(|(word, rect)| word == &view.names[0] && menu_rect.contains(rect.center()));
    let chosen = lower_row.expect("the chooser lists the lower region").1.center();
    view.button(chosen, true, egui::Modifiers::NONE);
    view.button(chosen, false, egui::Modifiers::NONE);
    assert!(view.state.choices.is_empty());
    assert_eq!(view.state.next_move.as_ref(), Some(&view.names[0]));
    view.frame(Vec::new(), egui::Modifiers::NONE);
    view.button(point, true, egui::Modifiers::NONE);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    assert!(view.state.next_move.is_none());
    view.frame(vec![egui::Event::PointerMoved(point + Vec2::splat(12.0))], egui::Modifiers::NONE);
    let released = point + Vec2::splat(32.0);
    view.frame(vec![egui::Event::PointerMoved(released)], egui::Modifiers::NONE);
    view.button(released, false, egui::Modifiers::NONE);
    assert_ne!(view.region(0).position, lower_before.position);
    assert_eq!(view.region(1), upper_before);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    assert!(view.state.moving.is_none());
}

#[test]
fn changing_modifiers_during_a_drag_keeps_the_region_chosen_on_press() {
    let mut view = TestCanvas::new();
    let point = view.overlap();
    view.button(point, true, egui::Modifiers::ALT);
    view.frame(vec![egui::Event::PointerMoved(point + Vec2::splat(12.0))], egui::Modifiers::NONE);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    assert_eq!(view.state.moving.as_ref().map(|moving| &moving.region), Some(&view.names[0]));
    assert!(view.state.choices.is_empty());
    view.button(point, false, egui::Modifiers::NONE);
    assert!(view.state.moving.is_none());
}

#[test]
fn a_press_and_move_in_one_frame_use_the_original_press_position() {
    let mut view = TestCanvas::new();
    let point = view.overlap();
    let moved = view.canvas.min + Vec2::splat(view.canvas.width() * 0.7);
    let pressed = egui::Event::PointerButton {
        pos: point,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::ALT,
    };
    let events = vec![egui::Event::PointerMoved(point), pressed, egui::Event::PointerMoved(moved)];
    view.frame(events, egui::Modifiers::ALT);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    let moving = view.state.moving.as_ref().unwrap();
    assert_eq!(moving.region, view.names[0]);
    assert_eq!(moving.start, point);
    view.button(moved, false, egui::Modifiers::ALT);
    assert!(view.state.moving.is_none());
}

#[test]
fn escape_clears_an_interrupted_drag_before_the_button_is_released() {
    let mut view = TestCanvas::new();
    let point = view.overlap();
    view.button(point, true, egui::Modifiers::NONE);
    view.frame(vec![egui::Event::PointerMoved(point + Vec2::splat(12.0))], egui::Modifiers::NONE);
    assert!(view.state.moving.as_ref().unwrap().corner.is_some());
    let escape = egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    view.frame(vec![escape], egui::Modifiers::NONE);
    assert!(view.state.moving.is_none());
    assert!(view.state.resizing.is_none());
}

#[test]
fn a_thumb_click_starts_no_resize_but_a_thumb_drag_marks_the_terrain_stale() {
    let mut view = TestCanvas::new();
    view.editor.preview_region = Some(view.names[0].clone());
    let thumb = view.canvas.min + view.canvas.size() * Vec2::splat(0.5);
    view.button(thumb, true, egui::Modifiers::NONE);
    assert_eq!(view.state.focused.as_ref(), Some(&view.names[0]));
    assert!(view.state.resizing.as_ref().is_some_and(|resizing| !resizing.changed));
    view.button(thumb, false, egui::Modifiers::NONE);
    assert!(view.state.resizing.is_none());
    assert!(!view.editor.needs_build, "a click on the thumb without a drag is not a resize");

    let width_before = view.region(0).width;
    view.button(thumb, true, egui::Modifiers::NONE);
    view.frame(vec![egui::Event::PointerMoved(thumb + Vec2::splat(12.0))], egui::Modifiers::NONE);
    let released = thumb + Vec2::splat(32.0);
    view.frame(vec![egui::Event::PointerMoved(released)], egui::Modifiers::NONE);
    view.button(released, false, egui::Modifiers::NONE);
    assert!(view.state.resizing.is_none());
    assert_ne!(view.region(0).width, width_before);
    assert!(view.editor.needs_build, "a real resize ends in RegionUpdated");
}
