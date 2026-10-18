//! `EditVariableDialog` (480 wide, "Edit Variable (Float)"): the variable's
//! name, Min and Max for Float, Int and Range (`NumericTextBox`es), the
//! choices for Choice and the string type for Undefined. Accept Changes
//! (Enter) checks the name and writes the rest, renaming the variable when
//! its name changed; Cancel (Escape) leaves it as it was. It opens from a
//! row's Edit… and after Create Variable makes one.

use egui::text::{CCursor, CCursorRange};
use egui::{Align, CursorIcon, Id, Key, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder, Vec2};
use nerthus_engine::automation::{VarType, Variable, VariableValue};
use serde_json::Value;

use crate::document::{variable_name_problem, VariableLimits};
use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::editors::dropdown_box;
use crate::ui::widgets;

/// The window (480) less its margin (12 each side).
const CONTENT_WIDTH: f32 = 480.0 - 24.0;
/// `NumericTextBox`'s `Minimum` and `Maximum` here (`RecreateUI`).
const LIMIT: f32 = 65535.0;
/// A `NumericTextBox`'s header (`MinWidth="96"`) and box (96 at least).
const NUMBER_COLUMN: f32 = 96.0;
/// `Thumb_OnDragDelta`: how far a point of dragging the header moves the
/// value.
const DRAG_PER_POINT: f32 = 0.01;
const INTEGER_DRAG_PER_POINT: f32 = 3.3;
/// `cboStringType`'s entries.
const STRING_TYPES: [&str; 5] = ["Standard", "Input", "Output", "Input (Optional)", "Output (Optional)"];
const ROW_HEIGHT: f32 = 26.0;

/// A `NumericTextBox`: its value and the text shown for it.
#[derive(Clone, Debug)]
struct NumberField {
    value: f32,
    text: String,
    integer: bool,
    /// The value when its header started being dragged, and how far it
    /// has been dragged.
    drag: Option<(f32, f32)>,
}

impl NumberField {
    fn new(value: f32, integer: bool) -> Self {
        let mut field = Self { value, text: String::new(), integer, drag: None };
        field.text = field.value_text();
        field
    }

    /// `GetValueAsText`: `0.00`, or `0` for integers.
    fn value_text(&self) -> String {
        if self.integer {
            let rounded = self.value.round() as i64;
            rounded.to_string()
        } else {
            format!("{:.2}", self.value)
        }
    }

    /// Whether the text was typed since the value last took it.
    fn typed(&self) -> bool {
        self.text != self.value_text()
    }

    /// `Txt_OnLostFocus`: the text as a number held to the limits, or the
    /// value's text again when it is not one.
    fn commit(&mut self) {
        if let Ok(number) = self.text.trim().parse::<f32>()
            && number.is_finite()
        {
            self.value = number.clamp(-LIMIT, LIMIT);
        }
        self.text = self.value_text();
    }
}

/// The dialog while it is open.
#[derive(Clone, Debug)]
pub struct EditVariable {
    key: String,
    var_type: VarType,
    name: String,
    min: NumberField,
    max: NumberField,
    /// `varChoices`: the choices joined with ", ".
    choices: String,
    string_type: usize,
    focused: bool,
}

impl EditVariable {
    /// `RecreateUI`: the fields from the variable.
    fn new(key: &str, variable: &Variable) -> Self {
        let limit = |name: &str| variable.json.get(name).map(VariableValue::from_json).unwrap_or(VariableValue::Null);
        let integer = variable.var_type == VarType::Int;
        let min_value = if integer { limit("Min").to_int32().unwrap_or(0) as f32 } else { limit("Min").to_single().unwrap_or(0.0) };
        let max_value = if integer { limit("Max").to_int32().unwrap_or(0) as f32 } else { limit("Max").to_single().unwrap_or(0.0) };
        let extra = variable.json.get("ExtraData");
        let list = extra.and_then(|extra| extra.get("$values").or(Some(extra))).and_then(Value::as_array);
        let options: Vec<&str> = list.map(|items| items.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let joined = options.join(", ");
        let choices = joined.trim().trim_matches(',').to_string();
        // `Int32.TryParse(ExtraData)`, else Standard.
        let extra_text = extra.map(VariableValue::from_json).map(|value| value.to_text());
        let string_type = extra_text.and_then(|text| text.parse::<usize>().ok()).unwrap_or(0);
        Self {
            key: key.to_string(),
            var_type: variable.var_type,
            name: key.to_string(),
            min: NumberField::new(min_value, integer),
            max: NumberField::new(max_value, integer),
            choices,
            string_type,
            focused: false,
        }
    }

    /// `BtnOK_OnClick`'s changes for the variable's type.
    fn limits(&self) -> VariableLimits {
        match self.var_type {
            VarType::Float | VarType::Range => VariableLimits::Float { min: self.min.value, max: self.max.value },
            VarType::Int => {
                let min = (self.min.value as f64).round_ties_even() as i32;
                let max = (self.max.value as f64).round_ties_even() as i32;
                VariableLimits::Int { min, max }
            }
            VarType::Choice => {
                let parts = self.choices.split(',');
                let trimmed = parts.map(str::trim);
                let choices = trimmed.filter(|choice| !choice.is_empty()).map(str::to_string).collect();
                VariableLimits::Choices(choices)
            }
            VarType::Undefined => VariableLimits::StringType(self.string_type),
            VarType::Bool | VarType::Color | VarType::Decoration => VariableLimits::None,
        }
    }
}

fn window_id() -> Id {
    Id::new("edit-variable")
}

/// `new EditVariableDialog { Variable = key }.ShowDialog()`.
pub fn open(editor: &Editor, key: &str) -> Option<EditVariable> {
    let variable = editor.document.automation().variables.get(key)?;
    Some(EditVariable::new(key, variable))
}

/// The dialog, while one is open.
pub fn show(ctx: &egui::Context, editor: &mut Editor, open_dialog: &mut Option<EditVariable>, commands: &mut Vec<Command>) {
    let Some(dialog) = open_dialog.as_mut() else {
        return;
    };
    if !editor.document.automation().variables.contains_key(&dialog.key) {
        *open_dialog = None;
        return;
    }
    let mut accept = false;
    let mut close = false;
    // `Window-Background`, `DockPanel Margin="12"`.
    let frame = widgets::dialog_frame(ctx).fill(theme::WINDOW_BACKGROUND);
    let modal = egui::Modal::new(window_id().with("modal")).frame(frame);
    let response = modal.show(ctx, |ui| {
        ui.set_width(CONTENT_WIDTH);
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        let title = format!("Edit Variable ({})", dialog.var_type.name());
        let caption = RichText::new(title).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS);
        ui.label(caption);
        ui.add_space(8.0);
        name_row(ui, dialog);
        match dialog.var_type {
            VarType::Float | VarType::Range | VarType::Int => {
                ui.add_space(8.0);
                number_field(ui, window_id().with("min"), "Min", &mut dialog.min);
                ui.add_space(8.0);
                number_field(ui, window_id().with("max"), "Max", &mut dialog.max);
            }
            VarType::Choice => {
                ui.add_space(8.0);
                choices_row(ui, dialog);
            }
            VarType::Undefined => {
                ui.add_space(8.0);
                string_type_row(ui, dialog);
            }
            VarType::Bool | VarType::Color | VarType::Decoration => {}
        }
        // The buttons (margin 8, 24, 8, 8), on the right, their text
        // centred in them.
        ui.add_space(24.0);
        ui.allocate_ui_with_layout(Vec2::new(CONTENT_WIDTH, ROW_HEIGHT), Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0);
            if dialog_button(ui, "Cancel", false).clicked() {
                close = true;
            }
            ui.add_space(8.0);
            if dialog_button(ui, "Accept Changes", true).clicked() {
                accept = true;
            }
        });
        ui.add_space(8.0);
    });
    // `IsDefault`: Enter accepts while nothing covers the dialog.
    let entered = ctx.input(|input| input.key_pressed(Key::Enter));
    if response.is_top_modal && entered {
        accept = true;
    }
    if response.should_close() {
        close = true;
    }
    if accept && !close {
        close = accept_changes(editor, dialog, commands);
    }
    if close {
        *open_dialog = None;
    }
}

/// `BtnOK_OnClick`: a reserved or taken name is refused with a message
/// (and the dialog stays); otherwise the changes are written. Numbers
/// typed but not yet taken count. Returns whether the dialog closes.
fn accept_changes(editor: &mut Editor, dialog: &mut EditVariable, commands: &mut Vec<Command>) -> bool {
    if let Some(problem) = variable_name_problem(&editor.document, &dialog.key, &dialog.name) {
        let caption = format!("Variable name [{}] not allowed", dialog.name);
        commands.push(Command::ShowMessage { caption, text: problem });
        return false;
    }
    for field in [&mut dialog.min, &mut dialog.max] {
        if field.typed() {
            field.commit();
        }
    }
    let numeric = matches!(dialog.var_type, VarType::Float | VarType::Range | VarType::Int);
    if numeric && (!dialog.min.value.is_finite() || !dialog.max.value.is_finite() || dialog.min.value > dialog.max.value) {
        let caption = "Variable limits not allowed".to_string();
        let text = "Min and Max must be finite numbers, with Min less than or equal to Max.".to_string();
        commands.push(Command::ShowMessage { caption, text });
        return false;
    }
    let limits = dialog.limits();
    editor.document.edit_variable(&dialog.key, &dialog.name, &limits);
    editor.changed();
    true
}

/// A field's row: `header`, and a `Ui` for the box from 8 after it to the
/// row's end, which centres the box down the row as the header is. In a
/// `DockPanel` row (`column` None) the header takes its own width with 8
/// round it (`Margin="8"`); a `NumericTextBox`'s is right-aligned in its
/// `column`. Returns the header's rect and that `Ui`.
fn field_row(ui: &mut Ui, header: &str, column: Option<f32>) -> (Rect, Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(CONTENT_WIDTH, ROW_HEIGHT), Sense::hover());
    let galley = ui.painter().layout_no_wrap(header.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT);
    let header_width = column.unwrap_or(8.0 + galley.size().x);
    let header_rect = Rect::from_min_size(rect.min, Vec2::new(header_width, rect.height()));
    let header_position = Pos2::new(header_rect.right() - galley.size().x, header_rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(header_position, galley, theme::LABEL_DEFAULT);
    let box_rect = Rect::from_min_max(Pos2::new(header_rect.right() + 8.0, rect.top()), rect.max);
    let box_ui = ui.new_child(UiBuilder::new().max_rect(box_rect).layout(Layout::left_to_right(Align::Center)));
    (header_rect, box_ui)
}

/// "Variable Name" and its box, focused with its text chosen when the
/// dialog opens.
fn name_row(ui: &mut Ui, dialog: &mut EditVariable) {
    let (_, mut box_ui) = field_row(ui, "Variable Name", None);
    let mut edit = egui::TextEdit::singleline(&mut dialog.name);
    edit = edit.id(window_id().with("name"));
    edit = edit.font(theme::monospace(12.0));
    edit = edit.desired_width(box_ui.available_width());
    let response = box_ui.add(edit);
    if !dialog.focused {
        // `EditVariableDialog_Loaded`: `Focus`, then `SelectAll`.
        response.request_focus();
        let length = dialog.name.chars().count();
        let mut state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
        let everything = CCursorRange::two(CCursor::new(0), CCursor::new(length));
        state.cursor.set_char_range(Some(everything));
        state.store(ui.ctx(), response.id);
        dialog.focused = true;
    }
}

/// `NumericTextBox`: the header right-aligned in its column (dragging it
/// moves the value), then the box 8 in, monospace in `Accent-Muted-Blue`
/// (`Accent-Muted-Green` for integers). Enter, Tab or leaving the box
/// takes its text.
fn number_field(ui: &mut Ui, id: Id, header: &str, field: &mut NumberField) {
    let (header_rect, mut box_ui) = field_row(ui, header, Some(NUMBER_COLUMN));
    // `thumb`: invisible over the header.
    let thumb = ui.interact(header_rect, id.with("thumb"), Sense::drag());
    if thumb.hovered() || thumb.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if thumb.drag_started() {
        field.drag = Some((field.value, 0.0));
    }
    if thumb.dragged()
        && let Some((start, dragged)) = field.drag
    {
        let distance = dragged + thumb.drag_delta().x;
        let per_point = if field.integer { INTEGER_DRAG_PER_POINT } else { DRAG_PER_POINT };
        field.value = (start + distance * per_point).clamp(-LIMIT, LIMIT);
        field.text = field.value_text();
        field.drag = Some((start, distance));
    }
    if thumb.drag_stopped() {
        field.drag = None;
    }
    let colour = if field.integer { theme::ACCENT_MUTED_GREEN } else { theme::ACCENT_MUTED_BLUE };
    let mut edit = egui::TextEdit::singleline(&mut field.text);
    edit = edit.id(id.with("text"));
    edit = edit.font(theme::monospace(12.0));
    edit = edit.text_color(colour);
    edit = edit.desired_width(NUMBER_COLUMN - 8.0);
    let response = box_ui.add(edit);
    if response.changed() {
        // `NumericOnlyBehavior.IsTextAllowed`: digits and a minus sign, and
        // a point for decimals.
        let allowed = |character: &char| character.is_ascii_digit() || *character == '-' || (*character == '.' && !field.integer);
        field.text = field.text.chars().filter(allowed).collect();
    }
    if response.lost_focus() {
        field.commit();
    }
}

/// `dockChoices`: "Choices" and its box, and the hint under it on the
/// right (10, at 0.7).
fn choices_row(ui: &mut Ui, dialog: &mut EditVariable) {
    let (_, mut box_ui) = field_row(ui, "Choices", None);
    let mut edit = egui::TextEdit::singleline(&mut dialog.choices);
    edit = edit.id(window_id().with("choices"));
    edit = edit.font(theme::monospace(12.0));
    edit = edit.desired_width(box_ui.available_width());
    box_ui.add(edit);
    ui.add_space(6.0);
    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
        let hint_colour = theme::LABEL_DEFAULT.gamma_multiply(0.7);
        let hint = RichText::new("Comma-separated list, only ASCII A-Z characters and spaces").font(theme::interface(10.0)).color(hint_colour);
        ui.label(hint);
    });
}

/// `cboStringType` (`DropdownX` "String Type"): its header in the theme's
/// column (`Column.Size`).
fn string_type_row(ui: &mut Ui, dialog: &mut EditVariable) {
    let entries: Vec<(String, String)> = STRING_TYPES.iter().enumerate().map(|(index, name)| (index.to_string(), name.to_string())).collect();
    let shown = STRING_TYPES.get(dialog.string_type).copied().unwrap_or("");
    let id = window_id().with("string-type");
    let column = theme::sizes(ui.ctx()).header_column;
    let chosen = dropdown_box(ui, id, Some("String Type"), column, ROW_HEIGHT, shown, &entries, theme::interface(12.0));
    if let Some(index) = chosen.and_then(|value| value.parse::<usize>().ok()) {
        dialog.string_type = index;
    }
}

/// `Button`, or with `important` `Button-Important`.
fn dialog_button(ui: &mut Ui, text: &str, important: bool) -> egui::Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = egui::Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(80.0, ROW_HEIGHT));
    ui.add(widget)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use egui::RawInput;
    use serde_json::json;

    /// The dialog on a variable of this kind, after a few frames at
    /// 1600 × 960 and 120%. Returns the context to read it back from.
    fn shown(kind: &str) -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        let key = editor.document.add_variable(kind);
        let mut dialog = open(&editor, &key);
        let mut commands = Vec::new();
        for _ in 0..3 {
            let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1333.0, 800.0))), ..Default::default() };
            let output = context.run_ui(input, |ui| show(ui.ctx(), &mut editor, &mut dialog, &mut commands));
            output.drop_without_applying_deltas();
        }
        assert!(dialog.is_some(), "the dialog closed");
        context
    }

    fn rect(context: &egui::Context, id: Id) -> Rect {
        context.read_response(id).map(|response| response.rect).unwrap_or_else(|| panic!("{id:?} was not shown"))
    }

    #[test]
    fn each_row_lays_its_header_out_as_gaea_s_and_its_box_level_with_it() {
        // The numbers' boxes 8 past their 96 header column, each level
        // with its header.
        let context = shown("Int");
        let name = rect(&context, window_id().with("name"));
        for limit in ["min", "max"] {
            let header = rect(&context, window_id().with(limit).with("thumb"));
            let number = rect(&context, window_id().with(limit).with("text"));
            assert!((header.width() - NUMBER_COLUMN).abs() < 0.5, "{limit} {header:?}");
            assert!(number.left() >= header.right() + 8.0 - 0.5, "{limit} {number:?} is not 8 past its header {header:?}");
            assert!((number.center().y - header.center().y).abs() < 0.5, "{limit} {number:?} is not level with its header {header:?}");
        }
        // "Variable Name" and "Choices" take their own widths with 8 round
        // them (`DockPanel` rows), so their boxes start apart.
        let context = shown("Choice");
        let choices = rect(&context, window_id().with("choices"));
        assert!(choices.left() < name.left(), "the choices {choices:?} start no nearer than the name {name:?}");
    }

    #[test]
    fn fields_come_from_the_variable_and_go_back_as_limits() {
        let choice = Variable::from_json(&json!({"Name": "Style", "Value": 0, "Type": "Choice", "ExtraData": {"$values": ["Simple", "Ridge"]}}));
        let mut dialog = EditVariable::new("Style", &choice);
        assert_eq!(dialog.choices, "Simple, Ridge");
        dialog.choices = " Simple ,, Ridge,Peak ".to_string();
        assert_eq!(dialog.limits(), VariableLimits::Choices(vec!["Simple".into(), "Ridge".into(), "Peak".into()]));

        let int = Variable::from_json(&json!({"Name": "Count", "Value": 1, "Type": "Int", "Min": 0, "Max": 10}));
        let mut dialog = EditVariable::new("Count", &int);
        assert_eq!((dialog.min.text.as_str(), dialog.max.text.as_str()), ("0", "10"));
        dialog.max.text = "99999".to_string();
        dialog.max.commit();
        assert_eq!(dialog.limits(), VariableLimits::Int { min: 0, max: 65535 });

        let text = Variable::from_json(&json!({"Name": "Path", "Value": "a", "ExtraData": "2"}));
        assert_eq!(EditVariable::new("Path", &text).limits(), VariableLimits::StringType(2));
    }

    #[test]
    fn inverted_variable_limits_keep_the_dialog_open_without_applying_changes() {
        for kind in ["Float", "Int", "Range"] {
            let mut editor = Editor::new(Document::new());
            let key = editor.document.add_variable(kind);
            let mut dialog = open(&editor, &key).unwrap();
            let before = editor.document.automation().variables[&key].json.clone();
            dialog.min.text = "5".to_string();
            dialog.max.text = "1".to_string();
            let mut commands = Vec::new();
            assert!(!accept_changes(&mut editor, &mut dialog, &mut commands));
            assert_eq!(editor.document.automation().variables[&key].json, before);
            assert!(matches!(commands.as_slice(), [Command::ShowMessage { .. }]));

            dialog.max.text = "10".to_string();
            commands.clear();
            assert!(accept_changes(&mut editor, &mut dialog, &mut commands));
            let variable = &editor.document.automation().variables[&key];
            assert_eq!(variable.json["Min"].as_f64(), Some(5.0));
            assert_eq!(variable.json["Max"].as_f64(), Some(10.0));
        }
    }

    fn dialog_frame(context: &egui::Context, editor: &mut Editor, dialog: &mut Option<EditVariable>, events: Vec<egui::Event>) {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1138.0, 640.0));
        let input = RawInput { events, screen_rect: Some(screen), ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui.ctx(), editor, dialog, &mut commands));
        output.drop_without_applying_deltas();
        assert!(commands.is_empty());
        assert!(dialog.is_some(), "choosing a string type must leave the dialog open");
    }

    #[test]
    fn string_type_popup_stays_above_the_modal_after_quick_clicks_and_drags() {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        let key = editor.document.add_variable("Undefined");
        let mut dialog = open(&editor, &key);
        for _ in 0..3 {
            dialog_frame(&context, &mut editor, &mut dialog, Vec::new());
        }
        let button_id = window_id().with("string-type").with("dropdown");
        let popup_id = button_id.with("popup");
        let button = rect(&context, button_id).center();
        let name = rect(&context, window_id().with("name")).center();
        let press = |position, pressed| egui::Event::PointerButton { pos: position, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        // Give the modal an earlier interaction, then open the list with a tap
        // whose press and release arrive in a single frame.
        for event in [egui::Event::PointerMoved(name), press(name, true), press(name, false)] {
            dialog_frame(&context, &mut editor, &mut dialog, vec![event]);
        }
        let tap = vec![egui::Event::PointerMoved(button), press(button, true), press(button, false)];
        dialog_frame(&context, &mut editor, &mut dialog, tap);
        for _ in 0..2 {
            dialog_frame(&context, &mut editor, &mut dialog, Vec::new());
        }
        assert!(egui::Popup::is_id_open(&context, popup_id));
        let popup = context.memory(|memory| memory.area_rect(popup_id)).unwrap();
        let popup_layer = egui::LayerId::new(egui::Order::Foreground, popup_id);
        let first_entry = Pos2::new(popup.center().x, popup.top() + 15.0);
        assert_eq!(context.layer_id_at(first_entry), Some(popup_layer));

        // Pressing and dragging in another field raises the modal. Its
        // child popup must keep its position above it for the whole gesture.
        let dragged = name + Vec2::new(60.0, 0.0);
        for event in [egui::Event::PointerMoved(name), press(name, true), egui::Event::PointerMoved(dragged), press(dragged, false)] {
            dialog_frame(&context, &mut editor, &mut dialog, vec![event]);
            assert_eq!(context.layer_id_at(first_entry), Some(popup_layer));
        }
        assert!(egui::Popup::is_id_open(&context, popup_id));
        let optional_output = Pos2::new(popup.center().x, popup.bottom() - 15.0);
        for event in [egui::Event::PointerMoved(optional_output), press(optional_output, true), press(optional_output, false)] {
            dialog_frame(&context, &mut editor, &mut dialog, vec![event]);
        }
        assert_eq!(dialog.unwrap().string_type, 4);
    }
}
