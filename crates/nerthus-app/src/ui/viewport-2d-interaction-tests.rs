use super::*;

struct TestView {
    context: egui::Context,
    editor: Editor,
    state: Viewport2DState,
    rect: Rect,
    outside_control: Rect,
    time: f64,
}

struct TestFrame {
    commands: Vec<Command>,
    outside_hovered: bool,
    outside_clicked: bool,
    outside_dragged: bool,
    toolbar_buttons: [Pos2; 2],
    words: Vec<String>,
}

impl TestView {
    fn new() -> Self {
        let context = crate::ui::widgets::fit_checks::context();
        context.set_pixels_per_point(1.2);
        context.global_style_mut(|style| style.interaction.tooltip_delay = 0.0);
        let mut document = crate::document::Document::new();
        let node = document.add_node("Crater", [26000.0, 26000.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.select(Some(node));
        let resolution = nerthus_engine::Resolution::square(2);
        let map = Map::from_data(resolution, vec![0.125, 0.25, 0.5, 0.75]);
        editor.outputs = vec![("Out".to_string(), Arc::new(map))];
        let mut state = Viewport2DState::default();
        state.docked = true;
        let rect = Rect::from_min_size(Pos2::new(200.0, 100.0), Vec2::splat(400.0));
        Self { context, editor, state, rect, outside_control: Rect::NOTHING, time: 0.0 }
    }

    fn frame(&mut self, events: Vec<egui::Event>) -> TestFrame {
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let input = egui::RawInput {
            screen_rect: Some(screen),
            time: Some(self.time),
            events,
            ..Default::default()
        };
        let mut frame = TestFrame {
            commands: Vec::new(),
            outside_hovered: false,
            outside_clicked: false,
            outside_dragged: false,
            toolbar_buttons: [Pos2::ZERO; 2],
            words: Vec::new(),
        };
        let mut output = self.context.run_ui(input, |ui| {
            let outside = ui.interact(self.outside_control, Id::new("outside-control"), Sense::click_and_drag());
            frame.outside_hovered = outside.hovered();
            frame.outside_clicked = outside.clicked();
            frame.outside_dragged = outside.dragged();

            let toolbar = view_toolbar_rect(ui, self.rect);
            let inner = toolbar.shrink(1.5).shrink2(Vec2::new(2.0, 4.0));
            let button = crate::ui::widgets::toolbar_button(ui);
            let first = Pos2::new(inner.center().x, inner.top() + button / 2.0);
            frame.toolbar_buttons = [first, first + Vec2::new(0.0, button + 2.0)];
            show_docked(ui, self.rect, &self.editor, &mut self.state, &mut frame.commands);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        for clipped in shapes {
            if let egui::Shape::Text(text) = clipped.shape {
                frame.words.push(text.galley.text().to_string());
            }
        }
        frame
    }

    fn settle(&mut self) -> TestFrame {
        self.frame(Vec::new());
        self.frame(Vec::new())
    }

    fn click(&mut self, pointer: Pos2, button: egui::PointerButton) -> TestFrame {
        let events = vec![egui::Event::PointerMoved(pointer), pointer_button(pointer, button, true)];
        self.frame(events);
        let events = vec![pointer_button(pointer, button, false)];
        self.frame(events)
    }

    fn gizmo(&mut self, x: f32, y: f32) {
        let node = self.editor.active_node().unwrap();
        self.editor.document.set_value(node, "X", serde_json::json!(x));
        self.editor.document.set_value(node, "Y", serde_json::json!(y));
        self.state.gizmo = Some(GizmoState::new(node, GizmoKind::Ts));
    }
}

fn pointer_button(pos: Pos2, button: egui::PointerButton, pressed: bool) -> egui::Event {
    egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE }
}

#[test]
fn picking_keeps_toolbar_hover_menus_and_secondary_context_menu_available() {
    for (index, tooltip, menu_word) in [(0, "Menu", "Pick Height/Color"), (1, "Mask Enhancement", "None / Raw")] {
        let mut view = TestView::new();
        view.state.docked_view.picking = true;
        let shown = view.settle();
        let pointer = shown.toolbar_buttons[index];
        view.frame(vec![egui::Event::PointerMoved(pointer)]);
        let hovered = view.settle();
        assert!(hovered.words.iter().any(|word| word == tooltip), "{tooltip} should show its tooltip");
        let response = view.context.read_response(Id::new("viewport-2d-docked")).unwrap();
        assert!(!response.hovered(), "the image should yield hover to {tooltip}");

        view.click(pointer, egui::PointerButton::Primary);
        let opened = view.settle();
        assert!(opened.words.iter().any(|word| word == menu_word), "{tooltip} should open its menu");
        assert!(view.state.is_picking());
        assert!(view.state.picked.is_none());
    }

    let mut view = TestView::new();
    view.state.docked_view.picking = true;
    view.settle();
    view.click(view.rect.center(), egui::PointerButton::Secondary);
    let opened = view.settle();
    assert!(opened.words.iter().any(|word| word == "Pick Height/Color"));
    assert!(view.state.is_picking());
    assert!(view.state.picked.is_none());
}

#[test]
fn primary_click_on_the_image_still_samples_and_ends_picking() {
    let mut view = TestView::new();
    view.state.docked_view.picking = true;
    view.settle();
    let pointer = view.rect.min + Vec2::splat(100.0);
    view.click(pointer, egui::PointerButton::Primary);
    assert!(!view.state.is_picking());
    let picked = view.state.picked.as_ref().expect("the image click samples a pixel");
    assert_eq!(picked.values, vec![0.125]);
}

#[test]
fn invisible_gizmo_ring_yields_to_outside_controls_and_visible_drags_still_finish() {
    let mut view = TestView::new();
    view.gizmo(0.0, 0.5);
    let outside = Pos2::new(view.rect.left() - 38.0, view.rect.center().y);
    view.outside_control = Rect::from_center_size(outside, Vec2::splat(28.0));
    view.settle();
    let hovered = view.frame(vec![egui::Event::PointerMoved(outside)]);
    assert!(hovered.outside_hovered);
    assert!(view.click(outside, egui::PointerButton::Primary).outside_clicked);

    let events = vec![pointer_button(outside, egui::PointerButton::Primary, true)];
    assert!(view.frame(events).commands.is_empty());
    let moved = outside + Vec2::new(20.0, 0.0);
    let dragged = view.frame(vec![egui::Event::PointerMoved(moved)]);
    assert!(dragged.outside_dragged);
    assert!(dragged.commands.is_empty());
    assert!(!view.state.gizmo_held());
    view.frame(vec![pointer_button(moved, egui::PointerButton::Primary, false)]);

    view.gizmo(0.5, 0.5);
    view.settle();
    let inside = view.rect.center();
    let events = vec![egui::Event::PointerMoved(inside), pointer_button(inside, egui::PointerButton::Primary, true)];
    let started = view.frame(events);
    let drag_started = started.commands.iter().any(|command| matches!(command, Command::GizmoEdit(edit) if edit.started));
    assert!(drag_started);
    let moved = inside + Vec2::new(30.0, 0.0);
    let dragged = view.frame(vec![egui::Event::PointerMoved(moved)]);
    let position_changed = dragged.commands.iter().any(|command| {
        let Command::GizmoEdit(edit) = command else {
            return false;
        };
        edit.values.iter().any(|(name, value)| name == "X" && value.as_f64().unwrap() > 0.5)
    });
    assert!(position_changed);
    let released = Pos2::new(view.rect.right() + 30.0, inside.y);
    let events = vec![egui::Event::PointerMoved(released), pointer_button(released, egui::PointerButton::Primary, false)];
    let finished = view.frame(events);
    let drag_finished = finished.commands.iter().any(|command| matches!(command, Command::GizmoEdit(edit) if edit.finished));
    assert!(drag_finished);
    assert!(!view.state.gizmo_held());
}

#[test]
fn invisible_measure_handle_yields_to_outside_controls_and_visible_handle_still_drags() {
    let mut view = TestView::new();
    view.state.docked_view.measuring = true;
    view.state.docked_view.fixed = false;
    view.state.docked_view.zoom = 2.0;
    view.state.docked_view.measure = [Pos2::new(0.5, 0.5), Pos2::new(0.8, 0.375)];
    let image = image_rect(view.rect, &view.state.docked_view);
    let outside = image.min + view.state.docked_view.measure[1].to_vec2() * image.width();
    assert!(outside.x > view.rect.right());
    view.outside_control = Rect::from_center_size(outside, Vec2::splat(28.0));
    let initial = view.state.docked_view.measure;
    view.settle();
    let hovered = view.frame(vec![egui::Event::PointerMoved(outside)]);
    assert!(hovered.outside_hovered);
    assert!(view.click(outside, egui::PointerButton::Primary).outside_clicked);
    view.frame(vec![pointer_button(outside, egui::PointerButton::Primary, true)]);
    let moved = outside - Vec2::new(60.0, 0.0);
    let dragged = view.frame(vec![egui::Event::PointerMoved(moved)]);
    assert!(dragged.outside_dragged);
    view.frame(vec![pointer_button(moved, egui::PointerButton::Primary, false)]);
    assert_eq!(view.state.docked_view.measure, initial);

    let inside = view.rect.center();
    view.frame(vec![egui::Event::PointerMoved(inside)]);
    let events = vec![pointer_button(inside, egui::PointerButton::Primary, true)];
    view.frame(events);
    let handle_id = Id::new(("measure-handle", 0_usize));
    let handle = view.context.read_response(handle_id).unwrap();
    assert!(handle.drag_started(), "the visible handle should capture its press: {handle:?}");
    let moved = inside + Vec2::new(30.0, 15.0);
    view.frame(vec![egui::Event::PointerMoved(moved)]);
    view.frame(vec![pointer_button(moved, egui::PointerButton::Primary, false)]);
    assert!(view.state.docked_view.measure[0].x > initial[0].x);
    assert!(view.state.docked_view.measure[0].y > initial[0].y);
    assert_eq!(view.state.docked_view.measure[1], initial[1]);
}

#[test]
fn toolbar_buttons_and_padding_take_input_before_an_overlapping_gizmo() {
    for index in 0..2 {
        let mut view = TestView::new();
        view.gizmo(0.94, 0.08);
        let shown = view.settle();
        let pointer = shown.toolbar_buttons[index];
        let clicked = view.click(pointer, egui::PointerButton::Primary);
        assert!(clicked.commands.is_empty());
        assert!(!view.state.gizmo_held());
        assert!(egui::Popup::is_any_open(&view.context));
    }

    let mut view = TestView::new();
    view.gizmo(0.94, 0.08);
    view.settle();
    let padding = Pos2::new(view.rect.right() - 42.0, view.rect.top() + 6.0);
    let events = vec![egui::Event::PointerMoved(padding), pointer_button(padding, egui::PointerButton::Primary, true)];
    view.frame(events);
    let moved = padding + Vec2::new(15.0, 0.0);
    let dragged = view.frame(vec![egui::Event::PointerMoved(moved)]);
    assert!(dragged.commands.is_empty());
    assert!(!view.state.gizmo_held());
    view.frame(vec![pointer_button(moved, egui::PointerButton::Primary, false)]);
}
