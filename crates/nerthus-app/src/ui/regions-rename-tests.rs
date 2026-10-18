use super::*;

struct RenameView {
    context: egui::Context,
    editor: Editor,
    state: RegionsState,
    interface: crate::ui::UiState,
    region_name: String,
    time: f64,
}

impl RenameView {
    fn new(accept_on_lost_focus: bool) -> Self {
        let context = crate::ui::widgets::fit_checks::context();
        let settings = theme::InterfaceSettings { accept_on_lost_focus, ..Default::default() };
        theme::set_settings(&context, settings);
        let mut document = crate::document::Document::new();
        let region_name = document.add_region([0.0, 0.0]);
        let editor = Editor::new(document);
        let state = RegionsState::default();
        let interface = crate::ui::UiState::default();
        Self { context, editor, state, interface, region_name, time: 0.0 }
    }

    fn open(&mut self) {
        let rename = Rename { region: self.region_name.clone(), text: self.region_name.clone(), focused: false };
        self.state.renaming = Some(rename);
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        self.time += 0.1;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let canvas = Rect::from_min_size(Pos2::new(100.0, 100.0), Vec2::splat(400.0));
        let input = egui::RawInput { screen_rect: Some(screen), time: Some(self.time), events, ..Default::default() };
        let mut commands = Vec::new();
        self.context.run_ui(input, |ui| {
            rename_popup(ui, &mut self.editor, &mut self.state, canvas, &mut commands);
            crate::ui::prompt::show(ui.ctx(), &mut self.interface, &mut commands);
        }).drop_without_applying_deltas();
        let mut window = bevy::window::Window::default();
        crate::ui::commands::run(commands, &mut self.editor, &mut self.interface, &mut window);
    }

    fn settle(&mut self) {
        self.frame(Vec::new());
        self.frame(Vec::new());
        self.frame(Vec::new());
    }
}

fn outside_click() -> Vec<egui::Event> {
    let pos = Pos2::new(600.0, 400.0);
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE },
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE },
    ]
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
}

#[test]
fn the_opening_menu_click_keeps_rename_open_and_selects_its_name() {
    for accept in [false, true] {
        let mut view = RenameView::new(accept);
        view.open();
        view.frame(outside_click());
        assert!(view.state.renaming.as_ref().is_some_and(|rename| rename.focused));

        view.settle();
        view.frame(vec![egui::Event::Text("  Highlands  ".to_string())]);
        assert_eq!(view.state.renaming.as_ref().unwrap().text, "  Highlands  ");
        view.editor.preview_region = Some(view.region_name.clone());
        view.frame(vec![key(egui::Key::Enter)]);
        assert!(view.state.renaming.is_none());
        assert_eq!(view.editor.document.regions()[0].name, "Highlands");
        assert_eq!(view.editor.preview_region.as_deref(), Some("Highlands"));
    }
}

#[test]
fn later_outside_clicks_follow_the_accept_on_lost_focus_setting() {
    for accept in [false, true] {
        let mut view = RenameView::new(accept);
        view.open();
        view.settle();
        view.frame(vec![egui::Event::Text("Highlands".to_string())]);
        view.frame(outside_click());
        assert!(view.state.renaming.is_none());
        let expected_name = if accept { "Highlands" } else { &view.region_name };
        assert_eq!(view.editor.document.regions()[0].name, expected_name);
    }
}

#[test]
fn a_duplicate_name_retries_after_the_message_without_losing_the_typed_name() {
    for accept in [false, true] {
        let mut view = RenameView::new(accept);
        let other_name = view.editor.document.add_region([0.25, 0.25]);
        let duplicate_name = other_name.to_uppercase();
        view.open();
        view.settle();
        view.frame(vec![egui::Event::Text(duplicate_name.clone())]);
        view.frame(vec![key(egui::Key::Enter)]);
        assert_eq!(view.interface.prompt.as_ref().unwrap().caption, "Duplicate Name");

        view.settle();
        assert_eq!(view.state.renaming.as_ref().unwrap().text, duplicate_name);
        view.frame(vec![key(egui::Key::Enter)]);
        assert!(view.interface.prompt.is_none());
        view.settle();
        assert_eq!(view.state.renaming.as_ref().unwrap().text, duplicate_name);
        assert!(view.interface.prompt.is_none());

        view.frame(vec![egui::Event::Text("Highlands".to_string())]);
        view.frame(vec![key(egui::Key::Enter)]);
        assert!(view.state.renaming.is_none());
        let region_names: Vec<String> = view.editor.document.regions().into_iter().map(|region| region.name).collect();
        assert_eq!(region_names, ["Highlands", &other_name]);
    }
}
