//! The recurring controls: toolbars (`Toolbar` border style), icon
//! buttons (`Button-IconOnly`), icon menus (`MenuItem Tag="Icon"`), menu
//! entries (`Default-Menu-Item`) with an icon, an accent and a shortcut,
//! dialogs' frames and buttons, selectable items and tooltips.

use egui::containers::menu::{MenuButton, MenuConfig, SubMenu};
use egui::{Button, Color32, CornerRadius, FontId, Frame, Id, Margin, Pos2, Rect, Response, RichText, ScrollArea, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetText};

use crate::icons;
use crate::theme;
use crate::ui::{menu_keys, title_bar};

/// `Toolbar.ButtonSize` as the size theme has it.
pub fn toolbar_button(ui: &Ui) -> f32 {
    theme::sizes(ui.ctx()).toolbar_button
}

/// A dialog button's least height.
pub const DIALOG_BUTTON_HEIGHT: f32 = 26.0;
/// Inside a dialog's border (`DockPanel Margin="12"`) and a tooltip's.
/// egui gives both the theme's `menu_margin`, which is 1 for the menus.
const DIALOG_MARGIN: i8 = 12;
const TOOLTIP_MARGIN: Margin = Margin::symmetric(6, 4);
/// `Default-Menu-Item`: rows 23 high (11px text, padding 2, 4), a 30 icon
/// column and a 20 arrow column; menus at least 190 wide.
const MENU_ROW_HEIGHT: f32 = 23.0;
const MENU_ICON_COLUMN: f32 = 30.0;
const MENU_ARROW_COLUMN: f32 = 20.0;
const MENU_MIN_WIDTH: f32 = 190.0;
/// A disabled row's icon, text and arrow are drawn at half opacity.
const MENU_DISABLED_OPACITY: f32 = 0.5;

/// Paints a toolbar's cladding (`Panel-Cladding`, 4px corners).
pub fn toolbar_background(ui: &Ui, rect: Rect, border: Color32, border_width: f32) {
    toolbar_background_corners(ui, rect, CornerRadius::same(4), border, border_width);
}

/// A toolbar's cladding with its own corners (the graph toolbar squares
/// the side the Data View joins).
pub fn toolbar_background_corners(ui: &Ui, rect: Rect, corners: CornerRadius, border: Color32, border_width: f32) {
    let painter = ui.painter();
    painter.rect_filled(rect, corners, theme::PANEL_CLADDING);
    let stroke = Stroke::new(border_width, border);
    painter.rect_stroke(rect, corners, stroke, StrokeKind::Inside);
}

/// A square icon button (`ToolbarButton-IconX`: `Toolbar.ButtonSize`, a 16
/// icon brightening over a rounded hover fill).
pub fn icon_button(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>) -> Response {
    let size = Vec2::splat(toolbar_button(ui));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    if response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
    } else if hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    let base = tint.unwrap_or(theme::ICON_DEFAULT);
    let color = if hovered && tint.is_none() { theme::LABEL_EMPHASIS } else { base };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color);
    response.tooltip(tooltip)
}

/// `Button-IconOnly`: an icon of `icon_size` in a button of `size`; under
/// the pointer a dark fill and the muted grey.
pub fn icon_button_sized(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, size: Vec2, icon_size: f32) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    if hovered {
        ui.painter().rect_filled(rect, 2.0, theme::BUTTON_HOVER);
    }
    let base = tint.unwrap_or(theme::LABEL_DEFAULT);
    let color = if hovered { theme::ACCENT_MUTED_GRAY } else { base };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icon_size));
    icons::paint(ui, icon_rect, icon, color);
    response.tooltip(tooltip)
}

/// A toggle shown as an icon button (`ToolbarToggleButton-IconX`): on, a
/// pressed fill with the accent's outline and a bright icon.
pub fn icon_toggle(ui: &mut Ui, icon: &str, tooltip: &str, on: bool, accent: Color32) -> Response {
    let size = Vec2::splat(toolbar_button(ui));
    icon_toggle_sized(ui, icon, tooltip, on, accent, size)
}

pub fn icon_toggle_sized(ui: &mut Ui, icon: &str, tooltip: &str, on: bool, accent: Color32, size: Vec2) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    let pressed = response.is_pointer_button_down_on();
    let fill = if pressed || (on && !hovered) {
        Some(theme::BUTTON_PRESSED)
    } else if hovered {
        Some(theme::BUTTON_HOVER)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, 4.0, fill);
    }
    if on {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, accent), StrokeKind::Inside);
    }
    let color = if (on || hovered) && !pressed { theme::LABEL_EMPHASIS } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, icon, color);
    response.tooltip(tooltip)
}

/// An icon that opens a menu (`MenuItem Tag="Icon"`): `Toolbar.ButtonSize`
/// square, the `chevron-down-right` mark in its corner, `Menu-TopLevel`
/// under the pointer.
pub fn icon_menu(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, contents: impl FnOnce(&mut Ui)) -> Response {
    let size = Vec2::splat(toolbar_button(ui));
    icon_menu_sized(ui, icon, tooltip, tint, size, contents)
}

pub fn icon_menu_sized(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, size: Vec2, contents: impl FnOnce(&mut Ui)) -> Response {
    let response = icon_menu_button(ui, icon, tint, size, contents);
    // `_iconMenuArrow`: 8 square, margin 1, bottom right.
    let arrow_origin = response.rect.right_bottom() - Vec2::splat(1.0 + 8.0);
    let arrow = Rect::from_min_size(arrow_origin, Vec2::splat(8.0));
    icons::paint(ui, arrow, "chevron-down-right", theme::ICON_DEFAULT);
    response.tooltip(tooltip)
}

/// An icon button that opens a menu, without the corner mark (a
/// `Button-IconOnly` with a context menu).
pub fn icon_dropdown(ui: &mut Ui, icon: &str, tooltip: &str, tint: Option<Color32>, size: Vec2, contents: impl FnOnce(&mut Ui)) -> Response {
    let response = icon_menu_button(ui, icon, tint, size, contents);
    response.tooltip(tooltip)
}

fn icon_menu_button(ui: &mut Ui, icon: &str, tint: Option<Color32>, size: Vec2, contents: impl FnOnce(&mut Ui)) -> Response {
    let color = tint.unwrap_or(theme::ICON_DEFAULT);
    let image = icons::image(icon, icons::DEFAULT_SIZE, color);
    let mut button = match image {
        Some(image) => Button::image(image),
        None => Button::new(icon.to_string()),
    };
    button = button.frame_when_inactive(false);
    button = button.min_size(size);
    button = button.corner_radius(4);
    let menu = MenuButton::from_button(button).config(menu_config());
    let scope = ui.scope(|ui| {
        let widgets = &mut ui.visuals_mut().widgets;
        widgets.hovered.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        widgets.hovered.bg_stroke = Stroke::NONE;
        widgets.open.weak_bg_fill = theme::MENU_BACKGROUND_HOVER;
        widgets.open.bg_stroke = Stroke::NONE;
        widgets.active.bg_stroke = Stroke::NONE;
        let (response, _) = menu.ui(ui, |ui| {
            menu_style(ui);
            contents(ui);
        });
        response
    });
    scope.inner
}

/// Menus close on a click inside.
pub fn menu_config() -> MenuConfig {
    MenuConfig::default()
}

/// Inside a menu: no gaps between rows, at least 190 wide. Notes the
/// menu as open for the keys (`menu_keys`).
pub fn menu_style(ui: &mut Ui) {
    menu_keys::menu(ui);
    let style = ui.style_mut();
    style.visuals.override_text_color = Some(theme::MENU_FOREGROUND);
    style.spacing.item_spacing = Vec2::ZERO;
    style.spacing.button_padding = egui::vec2(2.0, 4.0);
    ui.set_min_width(MENU_MIN_WIDTH);
}

/// The rows of a menu dropped from the main menu (`menu_rows_within`),
/// in at most the window's height under the title bar. The menu then
/// opens under the bar, rather than egui moving it up over the bar (and
/// its own title there) with its last rows cut off at the window's bottom.
pub fn menu_rows<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let room = ui.ctx().content_rect().height() - title_bar::HEIGHT;
    menu_rows_within(ui, room, contents)
}

/// The rows of a menu that opens beside `anchor` (the toolbox's category
/// menus, `RectAlign::RIGHT_START`), in at most the room there is beside
/// it (`room_beside`). The menu then fits beside its button, rather than
/// egui moving it up over the title bar with its last rows cut off at the
/// window's bottom.
pub fn menu_rows_beside<R>(ui: &mut Ui, anchor: Rect, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let room = room_beside(ui, anchor);
    menu_rows_within(ui, room, contents)
}

/// A menu's rows (`menu_style`) in at most `room` high with the menu's
/// frame, scrolling once they are taller. While they scroll, the bar's
/// handle shows, beside them (`scroll_bar_gutter`).
fn menu_rows_within<R>(ui: &mut Ui, room: f32, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = Frame::menu(ui.style()).total_margin().sum().y;
    let max_height = (room - frame).max(MENU_ROW_HEIGHT);
    // A popup is laid out in the size it took last (egui's 400 high when
    // it first opens): the rows may take all the room there is.
    ui.set_max_height(max_height);
    let style = ui.spacing().scroll;
    scroll_bar_gutter(ui);
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.dormant_handle_opacity = scroll.active_handle_opacity;
    let rows = ScrollArea::vertical().id_salt("menu-rows").max_height(max_height).show(ui, |ui| {
        // Not for scroll areas in the rows.
        ui.spacing_mut().scroll = style;
        menu_style(ui);
        contents(ui)
    });
    rows.inner
}

/// How a menu row's text is drawn.
#[derive(Clone, Copy, Default)]
struct RowText {
    /// The text's colour instead of `Menu-Foreground`.
    color: Option<Color32>,
    /// `Tag="HEADER"`: `Typography.Capitals="AllSmallCaps"`.
    small_caps: bool,
}

/// The row's text as laid out: small caps are capitals at 4/5 size.
fn row_galley(ui: &Ui, text: &str, style: RowText, color: Color32) -> std::sync::Arc<egui::Galley> {
    if style.small_caps {
        return ui.painter().layout_no_wrap(text.to_uppercase(), theme::interface(11.0 * 0.8), color);
    }
    ui.painter().layout_no_wrap(text.to_string(), theme::interface(11.0), color)
}

/// How wide a menu row of `content_width` is drawn. A popup first lays its
/// rows out in a sizing pass, offered `Spacing::default_area_size` (600)
/// across: there a row takes only what it needs (at least the menu's 190),
/// so the menu is as wide as its widest row; afterwards every row runs
/// across the menu.
pub fn menu_row_width(ui: &Ui, content_width: f32) -> f32 {
    let needed = content_width.max(MENU_MIN_WIDTH);
    if ui.is_sizing_pass() {
        return needed;
    }
    ui.available_width().max(needed)
}

/// A row's shortcut colour: `Menu-Gesture`, or, when the row is disabled,
/// its label's grey. The gesture's slate dimmed by half as well would all
/// but vanish on the menu (under 2:1).
fn shortcut_color(enabled: bool) -> Color32 {
    if enabled {
        theme::MENU_GESTURE
    } else {
        theme::MENU_FOREGROUND.gamma_multiply(MENU_DISABLED_OPACITY)
    }
}

/// One menu row: the hover fill (inset 1, corners 4), the icon centred in
/// its column, the text 2 past it (11px, white when highlighted), the
/// shortcut in monospace 10 at the right and, for a submenu (`submenu`,
/// `open` while it shows), the arrow. Highlighted under the pointer, or
/// with the keys on it (`menu_keys`).
fn menu_row(ui: &mut Ui, icon: Option<(&str, Color32)>, text: &str, shortcut: &str, enabled: bool, submenu: Option<Id>, open: bool) -> Response {
    menu_row_styled(ui, icon, text, RowText::default(), shortcut, enabled, submenu, open)
}

#[allow(clippy::too_many_arguments)]
fn menu_row_styled(ui: &mut Ui, icon: Option<(&str, Color32)>, text: &str, style: RowText, shortcut: &str, enabled: bool, submenu: Option<Id>, open: bool) -> Response {
    let text_width = row_galley(ui, text, style, Color32::WHITE).size().x;
    let shortcut_width = if shortcut.is_empty() { 0.0 } else { ui.painter().layout_no_wrap(shortcut.to_string(), theme::monospace(10.0), Color32::WHITE).size().x + 2.0 + 8.0 };
    let content_width = MENU_ICON_COLUMN + 2.0 + text_width + 2.0 + shortcut_width.max(MENU_ARROW_COLUMN);
    let width = menu_row_width(ui, content_width);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, MENU_ROW_HEIGHT), sense);
    let highlighted = if enabled {
        menu_keys::row(ui, &response, submenu) || open
    } else {
        menu_keys::menu(ui);
        false
    };
    if highlighted {
        ui.painter().rect_filled(rect.shrink(1.0), 4.0, theme::MENU_BACKGROUND_HOVER);
    }
    let opacity = if enabled { 1.0 } else { MENU_DISABLED_OPACITY };
    if let Some((name, color)) = icon {
        let icon_center = Pos2::new(rect.left() + MENU_ICON_COLUMN / 2.0, rect.center().y);
        let icon_rect = Rect::from_center_size(icon_center, Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, icon_rect, name, color.gamma_multiply(opacity));
    }
    let plain_color = if highlighted { theme::MENU_FOREGROUND_HOVER } else { theme::MENU_FOREGROUND };
    let text_color = style.color.unwrap_or(plain_color).gamma_multiply(opacity);
    let galley = row_galley(ui, text, style, text_color);
    let text_position = Pos2::new(rect.left() + MENU_ICON_COLUMN + 2.0, rect.center().y - galley.size().y / 2.0);
    ui.painter().galley(text_position, galley, text_color);
    if !shortcut.is_empty() {
        let shortcut_color = shortcut_color(enabled);
        let shortcut_galley = ui.painter().layout_no_wrap(shortcut.to_string(), theme::monospace(10.0), shortcut_color);
        let shortcut_position = Pos2::new(rect.right() - 8.0 - shortcut_galley.size().x, rect.center().y - shortcut_galley.size().y / 2.0);
        ui.painter().galley(shortcut_position, shortcut_galley, shortcut_color);
    }
    if submenu.is_some() {
        let arrow_center = Pos2::new(rect.right() - MENU_ARROW_COLUMN / 2.0, rect.center().y);
        let arrow_rect = Rect::from_center_size(arrow_center, Vec2::splat(icons::DEFAULT_SIZE));
        icons::paint(ui, arrow_rect, "chevron-right", theme::MENU_ICON.gamma_multiply(opacity));
    }
    response
}

/// A menu entry: icon (in its accent colour), text and shortcut.
pub fn menu_item(ui: &mut Ui, icon: Option<&str>, accent: Option<Color32>, text: &str, shortcut: &str) -> Response {
    menu_item_enabled(ui, icon, accent, text, shortcut, true)
}

pub fn menu_item_enabled(ui: &mut Ui, icon: Option<&str>, accent: Option<Color32>, text: &str, shortcut: &str, enabled: bool) -> Response {
    let tint = accent.unwrap_or(theme::MENU_ICON);
    let icon = icon.map(|name| (name, tint));
    let response = menu_row(ui, icon, text, shortcut, enabled, None, false);
    if response.clicked() {
        ui.close();
    }
    response
}

/// A checkable entry (`IsCheckable`): checked, the check (`Accent-Blue`)
/// stands where the icon goes.
pub fn check_item(ui: &mut Ui, checked: bool, text: &str, shortcut: &str) -> Response {
    let icon = if checked { Some(("check", theme::ACCENT_BLUE)) } else { None };
    let response = menu_row(ui, icon, text, shortcut, true, None, false);
    if response.clicked() {
        ui.close();
    }
    response
}

/// A menu entry with an icon that may be checked (`IsChecked`): then the
/// check in `Accent-Blue` takes the icon's place. Returns whether it was
/// clicked.
pub fn checked_icon_item(ui: &mut Ui, checked: bool, icon: Option<&str>, accent: Option<Color32>, text: &str, shortcut: &str) -> bool {
    let (shown_icon, shown_accent) = if checked { (Some("check"), Some(theme::ACCENT_BLUE)) } else { (icon, accent) };
    menu_item(ui, shown_icon, shown_accent, text, shortcut).clicked()
}

/// A greyed caption inside a menu (`IsEnabled="False"` headers).
pub fn menu_caption(ui: &mut Ui, text: &str) {
    menu_row(ui, None, text, "", false, None, false);
}

/// A submenu entry (`SubmenuHeader`): the row with its arrow, opening
/// `contents` beside it. Its rows scroll once there are more than fit
/// beside the row (`submenu_room`), as the graph search's "more..." can
/// have.
pub fn submenu<R>(ui: &mut Ui, icon: Option<&str>, accent: Option<Color32>, text: &str, contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let tint = accent.unwrap_or(theme::MENU_ICON);
    let icon = icon.map(|name| (name, tint));
    submenu_row(ui, contents, |ui, child, open| menu_row(ui, icon, text, "", true, Some(child), open))
}

/// A family header with its submenu (the toolbox's `Tag="HEADER"` items):
/// icon and small-caps text in `color`.
pub fn header_submenu<R>(ui: &mut Ui, icon: &str, color: Color32, text: &str, contents: impl FnOnce(&mut Ui) -> R) -> Option<R> {
    let style = RowText { color: Some(color), small_caps: true };
    submenu_row(ui, contents, |ui, child, open| menu_row_styled(ui, Some((icon, color)), text, style, "", true, Some(child), open))
}

/// The row `draw` draws (given its submenu's id and whether it is open)
/// and the submenu beside it. While the keys are in use, only they open
/// submenus.
fn submenu_row<R>(ui: &mut Ui, contents: impl FnOnce(&mut Ui) -> R, draw: impl FnOnce(&mut Ui, Id, bool) -> Response) -> Option<R> {
    let child = SubMenu::id_from_widget_id(ui.next_auto_id());
    let open = menu_keys::submenu_open(ui, child);
    let response = draw(ui, child, open);
    if !open && menu_keys::active(ui.ctx()) {
        return None;
    }
    // egui opens the submenu with the pointer anywhere on the row: of a
    // row scrolled partly out of its menu's view (`menu_rows`), only on
    // what shows. It opens it beside the row, which in a menu that
    // scrolls is past the scroll bar beside the rows.
    let mut row = response.clone();
    row.rect = response.interact_rect;
    let scrolled = ui.stack().iter().find(|stack| stack.kind() == Some(egui::UiKind::ScrollArea));
    if let Some(menu) = scrolled.and_then(|stack| stack.parent.as_deref()) {
        row.interact_rect.max.x = row.interact_rect.max.x.max(menu.max_rect.right());
    }
    let room = submenu_room(ui, &row);
    let shown = SubMenu::new().show(ui, &row, |ui| menu_rows_within(ui, room, contents));
    shown.map(|inner| inner.inner)
}

/// The most room a submenu has beside its row (`room_beside`).
fn submenu_room(ui: &Ui, row: &Response) -> f32 {
    // egui's anchor: the row, taller by half the submenu's frame each way.
    let frame = Frame::menu(ui.style()).total_margin().sum().y;
    room_beside(ui, row.interact_rect.expand2(Vec2::new(0.0, frame / 2.0)))
}

/// The most room a menu has beside `anchor` and under the title bar:
/// down from the anchor's top, where egui opens it first
/// (`RectAlign::RIGHT_START`), or up from the anchor's bottom
/// (`RIGHT_END`, or the same on the left), a pixel short against
/// rounding. No taller, the menu fits one of them, so egui opens it
/// beside the anchor rather than centred across it (over the bar) or,
/// when nothing fits, moved over its menu and still past the window's
/// edge.
fn room_beside(ui: &Ui, anchor: Rect) -> f32 {
    let window = ui.ctx().content_rect();
    let below = window.bottom() - anchor.top();
    let above = anchor.bottom() - (window.top() + title_bar::HEIGHT);
    below.max(above) - 1.0
}

/// `MenuItem.SeparatorStyleKey`: a full-width 1px rule (`Menu-Separator`
/// at 0.2) with 1 above and below.
pub fn menu_separator(ui: &mut Ui) {
    // On the sizing pass the rule claims no width: the rows set the menu's.
    let width = if ui.is_sizing_pass() { 0.0 } else { ui.available_width() };
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 3.0), Sense::hover());
    let y = rect.center().y;
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR.gamma_multiply(0.2));
    ui.painter().line_segment([Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)], stroke);
}

/// A dialog's frame: a popup's fill, border, corners and shadow, with
/// `DIALOG_MARGIN` inside.
pub fn dialog_frame(ctx: &egui::Context) -> Frame {
    Frame::popup(&ctx.global_style()).inner_margin(DIALOG_MARGIN)
}

/// A modal dialog in `dialog_frame`.
pub fn dialog(ctx: &egui::Context, id: Id) -> egui::Modal {
    egui::Modal::new(id).frame(dialog_frame(ctx))
}

/// A dialog's `Button`, or with `important` its `Button-Important`.
pub fn dialog_button(ui: &mut Ui, text: &str, important: bool) -> Response {
    let label = RichText::new(text).font(theme::interface(12.0)).color(theme::LABEL_EMPHASIS);
    let border = if important { theme::BUTTON_IMPORTANT_BORDER } else { theme::BUTTON_BORDER };
    let mut widget = Button::new(label);
    widget = widget.fill(theme::BUTTON_BACKGROUND);
    widget = widget.stroke(Stroke::new(1.0, border));
    widget = widget.min_size(Vec2::new(80.0, DIALOG_BUTTON_HEIGHT));
    ui.add(widget)
}

/// A row of dialog buttons, from the right, as high as they are (a layout
/// given the rest of the dialog would take it all), their text centred.
pub fn dialog_button_row<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    let row = Vec2::new(ui.available_width(), DIALOG_BUTTON_HEIGHT);
    ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), add_contents).inner
}

/// The tooltips, as egui's `on_hover_text` and its kin show them but
/// with `TOOLTIP_MARGIN` round them.
pub trait Tooltips {
    /// `text` under the widget while it is hovered and enabled.
    fn tooltip(self, text: impl Into<WidgetText>) -> Self;
    /// `text` beside the pointer while the widget is hovered and enabled.
    fn tooltip_at_pointer(self, text: impl Into<WidgetText>) -> Self;
    /// `text` under the widget while it is hovered and disabled.
    fn disabled_tooltip(self, text: impl Into<WidgetText>) -> Self;
    /// `add_contents` under the widget while it is hovered and enabled.
    fn tooltip_ui(self, add_contents: impl FnOnce(&mut Ui)) -> Self;
}

impl Tooltips for Response {
    fn tooltip(self, text: impl Into<WidgetText>) -> Self {
        show_tooltip(egui::Tooltip::for_enabled(&self), tooltip_text(text));
        self
    }

    fn tooltip_at_pointer(self, text: impl Into<WidgetText>) -> Self {
        show_tooltip(egui::Tooltip::for_enabled(&self).at_pointer().gap(12.0), tooltip_text(text));
        self
    }

    fn disabled_tooltip(self, text: impl Into<WidgetText>) -> Self {
        show_tooltip(egui::Tooltip::for_disabled(&self), tooltip_text(text));
        self
    }

    fn tooltip_ui(self, add_contents: impl FnOnce(&mut Ui)) -> Self {
        show_tooltip(egui::Tooltip::for_enabled(&self), add_contents);
        self
    }
}

fn show_tooltip(mut tooltip: egui::Tooltip<'_>, add_contents: impl FnOnce(&mut Ui)) {
    let frame = Frame::popup(&tooltip.popup.ctx().global_style()).inner_margin(TOOLTIP_MARGIN);
    tooltip.popup = tooltip.popup.frame(frame);
    tooltip.show(add_contents);
}

/// A tooltip's words, laid out as egui's are.
fn tooltip_text(text: impl Into<WidgetText>) -> impl FnOnce(&mut Ui) {
    let text = text.into();
    move |ui| {
        // Kept from shrinking as the area sizes itself.
        ui.set_max_width(ui.spacing().tooltip_width);
        ui.add(egui::Label::new(text));
    }
}

/// `Button-Link`: blue text (darker blue under the pointer) with the hand
/// cursor, inside `padding`.
pub fn link_button(ui: &mut Ui, text: &str, font: FontId, padding: Vec2) -> Response {
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, theme::ACCENT_BLUE);
    let (rect, response) = ui.allocate_exact_size(galley.size() + padding * 2.0, Sense::click());
    let color = if response.hovered() { theme::ACCENT_STRONG_BLUE } else { theme::ACCENT_BLUE };
    ui.painter().galley(rect.min + padding, galley, color);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A selectable item's text: when selected, the selection stroke's colour
/// (`Label-Emphasis`). egui would draw it so, but the theme's
/// `override_text_color` (`Label-Default`) beats its selected colour and
/// leaves grey text on the blue selection fill.
pub fn selectable_text(ui: &Ui, selected: bool, text: impl Into<WidgetText>) -> WidgetText {
    let text = text.into();
    if !selected {
        return text;
    }
    text.color(ui.visuals().selection.stroke.color)
}

/// `Ui::selectable_label` with readable selected text (`selectable_text`).
pub fn selectable_label(ui: &mut Ui, selected: bool, text: impl Into<WidgetText>) -> Response {
    let text = selectable_text(ui, selected, text);
    ui.selectable_label(selected, text)
}

/// `Ui::selectable_value` with readable selected text (`selectable_text`).
pub fn selectable_value<V: PartialEq>(ui: &mut Ui, current: &mut V, value: V, text: impl Into<WidgetText>) -> Response {
    let text = selectable_text(ui, *current == value, text);
    ui.selectable_value(current, value, text)
}

/// A rule across a vertical toolbar (`Separator` with margins `above` and
/// `below`, 2 in from each side).
pub fn toolbar_separator(ui: &mut Ui, above: f32, below: f32) {
    let width = ui.available_width() - 4.0;
    ui.add_space(above);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 1.0), Sense::hover());
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR.gamma_multiply(0.5));
    ui.painter().line_segment([rect.left_center(), rect.right_center()], stroke);
    ui.add_space(below);
}

/// How tall a vertical toolbar's two groups were when last laid out.
#[derive(Clone, Copy)]
struct ToolbarGroups {
    top: f32,
    bottom: f32,
}

/// Where a vertical toolbar's top group ends and its bottom group starts
/// (`vertical_toolbar`).
pub struct ToolbarDock {
    /// Where the bottom group starts, down from the column's top, to end
    /// at the foot: while the last layout found both groups fit.
    docked_at: Option<f32>,
    /// The column's top.
    start: f32,
    /// The top group's height and where the bottom one starts, once the
    /// top one ends.
    split: Option<(f32, f32)>,
}

impl ToolbarDock {
    /// Ends the top group: what is added after it docks at the foot of
    /// the toolbar, as far down as the room allows.
    pub fn bottom(&mut self, ui: &mut Ui) {
        let top = ui.cursor().top() - self.start;
        if let Some(docked_at) = self.docked_at {
            ui.add_space((docked_at - top).max(0.0));
        }
        self.split = Some((top, ui.cursor().top()));
    }
}

/// A vertical toolbar's buttons in `inner`, cut to the toolbar `clip`:
/// what `contents` adds before `ToolbarDock::bottom` from the top down,
/// what it adds after (in order down the toolbar) docked at its foot. In
/// a toolbar too short for both groups the bottom one follows the top one
/// and the column scrolls (fading at the edge with more), so no button is
/// drawn over another or outside the toolbar, or takes clicks there.
pub fn vertical_toolbar<R>(ui: &mut Ui, id_salt: &str, inner: Rect, clip: Rect, contents: impl FnOnce(&mut Ui, &mut ToolbarDock) -> R) -> R {
    let id = ui.id().with(id_salt);
    let measured: Option<ToolbarGroups> = ui.data(|data| data.get_temp(id));
    let fits = |groups: &ToolbarGroups| groups.top + groups.bottom <= inner.height();
    let docked_at = measured.filter(fits).map(|groups| inner.height() - groups.bottom);
    // The fades over the scrolled edges blend into the cladding.
    let cladding = egui::Frame::new().fill(theme::PANEL_CLADDING);
    let info = egui::UiStackInfo::new(egui::UiKind::Frame).with_frame(cladding);
    let layout = egui::Layout::top_down(egui::Align::Center);
    let builder = egui::UiBuilder::new().id_salt(id_salt).max_rect(inner).layout(layout).ui_stack_info(info);
    let mut column = ui.new_child(builder);
    column.shrink_clip_rect(clip);
    // However short the toolbar: not egui's least of 64 past its foot.
    let mut scroll = egui::ScrollArea::vertical().id_salt("toolbar-scroll").auto_shrink(false).min_scrolled_height(0.0);
    if docked_at.is_some() {
        // Grown to fit after a scroll: back to the top this frame.
        scroll = scroll.vertical_scroll_offset(0.0);
    }
    let (result, groups) = scroll.show(&mut column, |ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        let start = ui.cursor().top();
        let mut dock = ToolbarDock { docked_at, start, split: None };
        let result = contents(ui, &mut dock);
        let end = ui.cursor().top();
        let groups = match dock.split {
            Some((top, bottom_start)) => ToolbarGroups { top, bottom: end - bottom_start },
            None => ToolbarGroups { top: end - start, bottom: 0.0 },
        };
        (result, groups)
    }).inner;
    let changed = measured.is_none_or(|measured| (measured.top - groups.top).abs() > 0.5 || (measured.bottom - groups.bottom).abs() > 0.5);
    if changed {
        ui.data_mut(|data| data.insert_temp(id, groups));
        // Lay out again with the groups' heights known.
        ui.ctx().request_discard("toolbar groups measured");
        ui.ctx().request_repaint();
    }
    result
}

/// How an `EditablePopup`'s text box was left: Enter or Tab take the
/// text, Escape drops it, and a click elsewhere takes it only with Accept
/// values when clicking outside of editable field (`AcceptOnLostFocus`).
/// None while it still has the keyboard.
pub fn popup_outcome(ui: &Ui, response: &Response) -> Option<bool> {
    if !response.lost_focus() {
        return None;
    }
    let keys = ui.input(|input| (input.key_pressed(egui::Key::Enter), input.key_pressed(egui::Key::Tab), input.key_pressed(egui::Key::Escape)));
    let (entered, tabbed, escaped) = keys;
    if escaped {
        return Some(false);
    }
    if entered || tabbed {
        return Some(true);
    }
    Some(theme::settings(ui.ctx()).accept_on_lost_focus)
}

/// For the scroll areas shown in `ui`: while a floating scroll bar shows,
/// it gets a gutter of its own, as wide as the bar grows under the pointer
/// and its margin, so that lines laid out to the content's width end
/// before the bar rather than under it.
pub fn scroll_bar_gutter(ui: &mut Ui) {
    let scroll = &mut ui.spacing_mut().scroll;
    scroll.floating_allocated_width = scroll.bar_width + scroll.bar_inner_margin;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a frame of the test menu laid out: its button and, once open,
    /// the menu, its rows and the submenu.
    struct Shown {
        button: Rect,
        menu: Option<Rect>,
        rows: Vec<Rect>,
        submenu: Option<Rect>,
    }

    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let output = context.run_ui(egui::RawInput::default(), |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// One frame of a Graph-like menu (its button at `button_x`) on a
    /// window `screen_width` wide: rows with and without shortcuts, a
    /// separator and the Bake submenu above Bookmarks.
    fn menu_frame(context: &egui::Context, screen_width: f32, button_x: f32, events: Vec<egui::Event>) -> Shown {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(screen_width, 700.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut shown = Shown { button: Rect::NOTHING, menu: None, rows: Vec::new(), submenu: None };
        let output = context.run_ui(input, |ui| {
            let slot = Rect::from_min_size(Pos2::new(button_x, 0.0), Vec2::new(60.0, 24.0));
            let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(slot));
            let menu = MenuButton::new("Graph").config(menu_config());
            let (response, _) = menu.ui(&mut bar, |ui| {
                menu_style(ui);
                let mut rows = vec![menu_item(ui, None, None, "Save as...", "Ctrl Shift S").rect];
                rows.push(menu_item(ui, None, None, "Convert port to portals", "Ctrl Shift Alt P").rect);
                menu_separator(ui);
                let bake = submenu(ui, None, None, "Bake", |ui| {
                    menu_style(ui);
                    menu_item(ui, None, None, "Bake Selected", "");
                    menu_item(ui, None, None, "16384 x 16384", "");
                    ui.min_rect()
                });
                rows.push(menu_item(ui, None, None, "Bookmarks", "").rect);
                if !ui.is_sizing_pass() {
                    shown.menu = Some(ui.min_rect());
                    shown.rows = rows;
                    shown.submenu = bake;
                }
            });
            shown.button = response.rect;
        });
        output.drop_without_applying_deltas();
        shown
    }

    fn click(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE }
    }

    /// Opens the menu and hovers its Bake row: the menu and the submenu
    /// as laid out once both have been sized.
    fn open_bake(screen_width: f32, button_x: f32) -> (Rect, Vec<Rect>, Rect) {
        let context = themed_context();
        let button = menu_frame(&context, screen_width, button_x, Vec::new()).button.center();
        menu_frame(&context, screen_width, button_x, vec![egui::Event::PointerMoved(button), click(button, true)]);
        menu_frame(&context, screen_width, button_x, vec![click(button, false)]);
        let mut shown = menu_frame(&context, screen_width, button_x, Vec::new());
        for _ in 0..2 {
            shown = menu_frame(&context, screen_width, button_x, Vec::new());
        }
        let menu = shown.menu.expect("the menu opens");
        let bake = Pos2::new(menu.center().x, shown.rows[1].bottom() + 3.0 + MENU_ROW_HEIGHT / 2.0);
        menu_frame(&context, screen_width, button_x, vec![egui::Event::PointerMoved(bake)]);
        for _ in 0..3 {
            shown = menu_frame(&context, screen_width, button_x, Vec::new());
        }
        let submenu = shown.submenu.expect("hovering Bake opens its submenu");
        (shown.menu.unwrap(), shown.rows, submenu)
    }

    #[test]
    fn a_menu_is_as_wide_as_its_widest_row() {
        let (menu, rows, _) = open_bake(1000.0, 200.0);
        // Not egui's 600 for an area of unknown size.
        assert!(menu.width() < 300.0, "the menu is {} wide", menu.width());
        assert!(menu.width() >= MENU_MIN_WIDTH);
        // Every row runs across the menu.
        assert!(rows.iter().all(|row| row.width() == menu.width()));
        // Under its button.
        assert!(menu.left() >= 200.0 - 10.0 && menu.left() <= 200.0 + 10.0, "the menu starts at {}", menu.left());
    }

    #[test]
    fn a_submenu_opens_beside_its_menu() {
        let (menu, _, submenu) = open_bake(1000.0, 200.0);
        assert!(submenu.left() >= menu.right(), "{submenu:?} covers {menu:?}");
    }

    #[test]
    fn a_submenu_with_no_room_on_the_right_opens_on_the_left() {
        let (menu, _, submenu) = open_bake(640.0, 520.0);
        assert!(submenu.right() <= menu.left(), "{submenu:?} covers {menu:?}");
    }

    /// The WCAG contrast of `color`, laid over `background` as egui blends
    /// (in gamma space), against `background`.
    fn contrast(color: Color32, background: Color32) -> f32 {
        let luminance = |color: Color32| {
            let [red, green, blue] = theme::sc_rgb(color);
            0.2126 * red + 0.7152 * green + 0.0722 * blue
        };
        let front = luminance(background.blend(color));
        let back = luminance(background);
        (front.max(back) + 0.05) / (front.min(back) + 0.05)
    }

    /// A button of the test toolbar: where it was laid out, where it
    /// takes clicks and whether it was clicked.
    struct ToolbarButton {
        rect: Rect,
        interact: Rect,
        clicked: bool,
    }

    /// The test toolbar's inner rect, `height` high.
    fn toolbar_inner(context: &egui::Context, height: f32) -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), Vec2::new(theme::sizes(context).toolbar_button, height))
    }

    /// One frame of a vertical toolbar in `inner`: three buttons and a
    /// separator at the top, two buttons docked at the bottom.
    fn toolbar_frame(context: &egui::Context, inner: Rect, events: Vec<egui::Event>) -> Vec<ToolbarButton> {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 700.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut buttons = Vec::new();
        let output = context.run_ui(input, |ui| {
            buttons.clear();
            vertical_toolbar(ui, "test-toolbar", inner, inner.expand(2.0), |ui, dock| {
                let mut button = |ui: &mut Ui| {
                    let response = icon_button(ui, "sun", "", None);
                    buttons.push(ToolbarButton { rect: response.rect, interact: response.interact_rect, clicked: response.clicked() });
                };
                for _ in 0..3 {
                    button(ui);
                }
                toolbar_separator(ui, 8.0, 8.0);
                dock.bottom(ui);
                for _ in 0..2 {
                    button(ui);
                }
            });
        });
        output.drop_without_applying_deltas();
        buttons
    }

    /// The test toolbar's buttons and separator one under another.
    fn toolbar_content(context: &egui::Context) -> f32 {
        5.0 * theme::sizes(context).toolbar_button + 17.0
    }

    /// A click at `at` over three frames: onto it, pressed, released. The
    /// buttons clicked.
    fn click_toolbar(context: &egui::Context, inner: Rect, at: Pos2) -> Vec<usize> {
        toolbar_frame(context, inner, vec![egui::Event::PointerMoved(at)]);
        toolbar_frame(context, inner, vec![click(at, true)]);
        let buttons = toolbar_frame(context, inner, vec![click(at, false)]);
        buttons.iter().enumerate().filter(|(_, button)| button.clicked).map(|(index, _)| index).collect()
    }

    fn assert_apart_and_inside(buttons: &[ToolbarButton], inner: Rect, case: &str) {
        assert_eq!(buttons.len(), 5, "{case}");
        for pair in buttons.windows(2) {
            assert!(pair[0].rect.bottom() <= pair[1].rect.top() + 0.01, "{case}: {:?} over {:?}", pair[0].rect, pair[1].rect);
        }
        for button in buttons.iter().filter(|button| button.interact.is_positive()) {
            let inside = button.interact.top() >= inner.top() - 0.01 && button.interact.bottom() <= inner.bottom() + 0.01;
            assert!(inside, "{case}: a button takes clicks at {:?}, outside {inner:?}", button.interact);
        }
    }

    #[test]
    fn a_toolbar_with_room_docks_its_bottom_group_at_its_foot() {
        let context = themed_context();
        let inner = toolbar_inner(&context, 400.0);
        let buttons = toolbar_frame(&context, inner, Vec::new());
        assert_apart_and_inside(&buttons, inner, "400");
        assert!((buttons[0].rect.top() - inner.top()).abs() < 0.5, "the top group starts at {}", buttons[0].rect.top());
        assert!((buttons[4].rect.bottom() - inner.bottom()).abs() < 0.5, "the bottom group ends at {}", buttons[4].rect.bottom());
        assert!(buttons[3].rect.top() - buttons[2].rect.bottom() > 400.0 - toolbar_content(&context));
    }

    #[test]
    fn a_short_toolbar_never_draws_a_button_over_another_or_outside_it() {
        let context = themed_context();
        // As a splitter drags it shorter, then taller again.
        let heights = (40..=400).rev().step_by(9).chain((40..=400).step_by(9));
        for height in heights {
            let inner = toolbar_inner(&context, height as f32);
            let buttons = toolbar_frame(&context, inner, Vec::new());
            assert_apart_and_inside(&buttons, inner, &height.to_string());
            if height as f32 >= toolbar_content(&context) {
                assert!((buttons[4].rect.bottom() - inner.bottom()).abs() < 0.5, "{height}: room for both, the bottom group ends at {}", buttons[4].rect.bottom());
            } else {
                assert!(buttons[3].rect.top() - buttons[2].rect.bottom() <= 17.01, "{height}: the bottom group does not follow the top one");
            }
        }
    }

    #[test]
    fn a_short_toolbar_scrolls_to_its_bottom_group() {
        let context = themed_context();
        let inner = toolbar_inner(&context, 80.0);
        let last = toolbar_frame(&context, inner, Vec::new())[4].rect;
        assert!(last.top() >= inner.bottom(), "the last button starts out of sight, at {last:?}");
        // Where it is laid out, under the toolbar, it takes no click.
        assert_eq!(click_toolbar(&context, inner, last.center()), Vec::<usize>::new());
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
        toolbar_frame(&context, inner, vec![egui::Event::PointerMoved(inner.center())]);
        toolbar_frame(&context, inner, vec![wheel]);
        for _ in 0..60 {
            toolbar_frame(&context, inner, Vec::new());
        }
        let buttons = toolbar_frame(&context, inner, Vec::new());
        assert_apart_and_inside(&buttons, inner, "scrolled");
        let last = buttons[4].rect;
        assert!((last.bottom() - inner.bottom()).abs() < 0.5, "scrolled to the end, the last button ends at {}", last.bottom());
        assert_eq!(click_toolbar(&context, inner, last.center()), [4]);
    }

    #[test]
    fn shortcuts_stay_legible_when_disabled() {
        let enabled = contrast(shortcut_color(true), theme::MENU_BACKGROUND);
        let disabled = contrast(shortcut_color(false), theme::MENU_BACKGROUND);
        assert!(enabled >= 3.0, "an enabled shortcut is {enabled:.2}:1");
        assert!(disabled >= 3.0, "a disabled shortcut is {disabled:.2}:1");
    }

    /// The colour each text in `shapes` was painted in, as epaint resolves
    /// it: the galley's own colour, or the shape's for a placeholder.
    fn text_colors(shapes: &[egui::Shape], colors: &mut Vec<(String, Color32)>) {
        for shape in shapes {
            match shape {
                egui::Shape::Vec(shapes) => text_colors(shapes, colors),
                egui::Shape::Text(text) => {
                    let format = text.galley.job.sections.first().map(|section| section.format.color);
                    let color = match format {
                        Some(color) if color != Color32::PLACEHOLDER => color,
                        _ => text.fallback_color,
                    };
                    colors.push((text.galley.text().to_string(), text.override_text_color.unwrap_or(color)));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn selected_items_are_legible_on_the_selection_fill() {
        let context = themed_context();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 300.0));
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut output = context.run_ui(input, |ui| {
            selectable_label(ui, true, "Resolution");
            selectable_label(ui, false, "Build");
            let mut profile = 1;
            selectable_value(ui, &mut profile, 1, "Test Profile");
            selectable_value(ui, &mut profile, 2, "Other Profile");
            let text = selectable_text(ui, true, "Terrain");
            ui.add(Button::selectable(true, text));
        });
        let shapes: Vec<egui::Shape> = std::mem::take(&mut output.shapes).into_iter().map(|clipped| clipped.shape).collect();
        output.drop_without_applying_deltas();
        let mut colors = Vec::new();
        text_colors(&shapes, &mut colors);
        let color = |wanted: &str| colors.iter().find(|(text, _)| text == wanted).map(|(_, color)| *color).expect(wanted);
        for selected in ["Resolution", "Test Profile", "Terrain"] {
            assert_eq!(color(selected), theme::LABEL_EMPHASIS, "{selected}");
            // `Label-Default` there was 1.05:1.
            let legible = contrast(color(selected), theme::TEXTBOX_FOCUSED_BORDER);
            assert!(legible > 2.0, "{selected} is {legible:.2}:1 on the selection");
        }
        for unselected in ["Build", "Other Profile"] {
            assert_eq!(color(unselected), theme::LABEL_DEFAULT, "{unselected}");
        }
    }

    /// A frame of a File-like menu with the keys taken as the app takes
    /// them: what had the focus and was highlighted, what was clicked.
    #[derive(Default)]
    struct Keyed {
        button: Pos2,
        open: bool,
        submenu: bool,
        focused: Option<&'static str>,
        highlighted: Vec<&'static str>,
        clicked: Vec<&'static str>,
        rows: Vec<(&'static str, Rect)>,
    }

    fn keyed_frame(context: &egui::Context, events: Vec<egui::Event>) -> Keyed {
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let keyed = std::cell::RefCell::new(Keyed::default());
        let note_row = |name: &'static str, rect: Rect, focused: bool, clicked: bool| {
            let mut keyed = keyed.borrow_mut();
            if focused {
                keyed.focused = Some(name);
            }
            if clicked {
                keyed.clicked.push(name);
            }
            keyed.rows.push((name, rect));
        };
        let note = |name: &'static str, response: &Response| note_row(name, response.rect, response.has_focus(), response.clicked());
        let output = context.run_ui(input, |ui| {
            menu_keys::begin(ui.ctx());
            let slot = Rect::from_min_size(Pos2::new(200.0, 0.0), Vec2::new(60.0, 24.0));
            let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(slot));
            let (response, inner) = MenuButton::new("File").config(menu_config()).ui(&mut bar, |ui| {
                menu_style(ui);
                note("New", &menu_item(ui, None, None, "New", "Ctrl N"));
                note("Open", &menu_item(ui, None, None, "Open", ""));
                menu_separator(ui);
                menu_caption(ui, "Recent Files");
                // The submenu's row, as `submenu` lays it out.
                let (row, at, width) = (ui.next_auto_id(), ui.cursor().min, ui.available_width());
                let shown = submenu(ui, None, None, "Recent", |ui| {
                    note("One", &menu_item(ui, None, None, "One", ""));
                    note("Two", &menu_item(ui, None, None, "Two", ""));
                });
                note_row("Recent", Rect::from_min_size(at, Vec2::new(width, MENU_ROW_HEIGHT)), ui.memory(|memory| memory.has_focus(row)), false);
                keyed.borrow_mut().submenu = shown.is_some();
                note("Share", &menu_item_enabled(ui, None, None, "Share", "", false));
                note("Exit", &menu_item(ui, None, None, "Exit", ""));
            });
            let mut keyed = keyed.borrow_mut();
            keyed.button = response.rect.center();
            keyed.open = inner.is_some();
        });
        // The fills painted (a highlighted row's is inset 1; the menu
        // fades in, so its colour is not yet the theme's).
        let fills: Vec<Rect> = output.shapes.iter().filter_map(|clipped| match &clipped.shape {
            egui::Shape::Rect(rect) if rect.fill != Color32::TRANSPARENT => Some(rect.rect),
            _ => None,
        }).collect();
        output.drop_without_applying_deltas();
        let mut keyed = keyed.into_inner();
        keyed.highlighted = keyed.rows.iter().filter(|(_, rect)| fills.contains(&rect.shrink(1.0))).map(|(name, _)| *name).collect();
        keyed
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
    }

    fn press(context: &egui::Context, pressed: egui::Key) -> Keyed {
        let keyed = keyed_frame(context, vec![key(pressed, egui::Modifiers::NONE)]);
        // The frames after, as the app draws them.
        keyed_frame(context, Vec::new());
        let settled = keyed_frame(context, Vec::new());
        Keyed { clicked: keyed.clicked, ..settled }
    }

    /// Opens the menu with a click and leaves the pointer at `rest`.
    fn opened(rest: Pos2) -> egui::Context {
        let context = themed_context();
        let button = keyed_frame(&context, Vec::new()).button;
        keyed_frame(&context, vec![egui::Event::PointerMoved(button), click(button, true)]);
        keyed_frame(&context, vec![click(button, false)]);
        keyed_frame(&context, vec![egui::Event::PointerMoved(rest)]);
        let shown = keyed_frame(&context, Vec::new());
        assert!(shown.open, "the menu opens");
        context
    }

    #[test]
    fn the_arrows_move_a_highlight_through_the_rows_and_enter_clicks_it() {
        // The pointer off the menu, up in the title bar.
        let context = opened(Pos2::new(600.0, 5.0));
        assert_eq!(keyed_frame(&context, Vec::new()).highlighted, Vec::<&str>::new());
        let shown = press(&context, egui::Key::ArrowDown);
        assert_eq!(shown.focused, Some("New"));
        assert_eq!(shown.highlighted, ["New"]);
        // The caption and the disabled row are passed over.
        assert_eq!(press(&context, egui::Key::ArrowDown).focused, Some("Open"));
        assert_eq!(press(&context, egui::Key::ArrowDown).highlighted, ["Recent"]);
        assert_eq!(press(&context, egui::Key::ArrowDown).focused, Some("Exit"));
        // Round to the first, and back.
        assert_eq!(press(&context, egui::Key::ArrowDown).focused, Some("New"));
        assert_eq!(press(&context, egui::Key::ArrowUp).focused, Some("Exit"));
        assert_eq!(press(&context, egui::Key::Home).focused, Some("New"));
        let clicked = press(&context, egui::Key::Enter);
        assert_eq!(clicked.clicked, ["New"]);
        assert!(!clicked.open, "the menu closes on the click");
    }

    #[test]
    fn right_opens_a_submenu_on_its_first_row_and_left_closes_it() {
        let context = opened(Pos2::new(600.0, 5.0));
        for _ in 0..3 {
            press(&context, egui::Key::ArrowDown);
        }
        let shown = press(&context, egui::Key::ArrowRight);
        assert!(shown.submenu, "Right opens Recent");
        assert_eq!(shown.focused, Some("One"));
        // The submenu's row stays highlighted with its own.
        assert_eq!(shown.highlighted, ["One", "Recent"]);
        assert_eq!(press(&context, egui::Key::ArrowDown).focused, Some("Two"));
        let back = press(&context, egui::Key::ArrowLeft);
        assert!(!back.submenu && back.open);
        assert_eq!(back.focused, Some("Recent"));
        // Enter opens it too; Escape closes only it, then the menu.
        assert_eq!(press(&context, egui::Key::Enter).focused, Some("One"));
        let escaped = press(&context, egui::Key::Escape);
        assert!(!escaped.submenu && escaped.open);
        assert_eq!(escaped.focused, Some("Recent"));
        assert!(!press(&context, egui::Key::Escape).open);
    }

    #[test]
    fn tab_steps_like_down_and_enter_clicks_a_row_in_a_submenu() {
        let context = opened(Pos2::new(600.0, 5.0));
        press(&context, egui::Key::Tab);
        assert_eq!(press(&context, egui::Key::Tab).focused, Some("Open"));
        let back = keyed_frame(&context, vec![key(egui::Key::Tab, egui::Modifiers::SHIFT)]);
        assert_eq!(back.focused, Some("New"));
        press(&context, egui::Key::ArrowUp);
        press(&context, egui::Key::ArrowUp);
        press(&context, egui::Key::ArrowRight);
        let clicked = press(&context, egui::Key::Space);
        assert_eq!(clicked.clicked, ["One"]);
        assert!(!clicked.open);
    }

    /// The long test menu's rows, and its submenu's.
    const LONG_ROWS: usize = 40;
    /// A window too short for them.
    const SHORT_SCREEN: Vec2 = Vec2::new(1000.0, 300.0);

    /// A frame of the long test menu: the menu and its submenu as their
    /// areas were laid out, the menu's rows and the one with the focus.
    struct Long {
        button: Rect,
        menu: Option<Rect>,
        submenu: Option<Rect>,
        rows: Vec<Rect>,
        focused: Option<usize>,
    }

    /// One frame of a menu dropped from a button at the bottom of the
    /// title bar on `SHORT_SCREEN`, with the keys taken as the app takes
    /// them: `LONG_ROWS` rows, and before the `more`th a submenu (More)
    /// of as many.
    fn long_frame(context: &egui::Context, more: usize, events: Vec<egui::Event>) -> Long {
        let screen = Rect::from_min_size(Pos2::ZERO, SHORT_SCREEN);
        let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
        let mut long = Long { button: Rect::NOTHING, menu: None, submenu: None, rows: Vec::new(), focused: None };
        let mut submenu_id = None;
        let output = context.run_ui(input, |ui| {
            menu_keys::begin(ui.ctx());
            let slot = Rect::from_min_size(Pos2::new(200.0, title_bar::HEIGHT - 24.0), Vec2::new(60.0, 24.0));
            let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(slot));
            let (response, inner) = MenuButton::new("Long").config(menu_config()).ui(&mut bar, |ui| {
                menu_rows(ui, |ui| {
                    for row in 0..LONG_ROWS {
                        if row == more {
                            let id = ui.next_auto_id();
                            let shown = submenu(ui, None, None, "More", |ui| {
                                for index in 0..LONG_ROWS {
                                    menu_item(ui, None, None, &format!("More {index}"), "");
                                }
                            });
                            submenu_id = shown.map(|_| SubMenu::id_from_widget_id(id));
                        }
                        let response = menu_item(ui, None, None, &format!("Row {row}"), "");
                        if response.has_focus() {
                            long.focused = Some(row);
                        }
                        long.rows.push(response.rect);
                    }
                });
            });
            long.button = response.rect;
            long.menu = inner.map(|inner| inner.response.rect);
        });
        output.drop_without_applying_deltas();
        long.submenu = submenu_id.and_then(|id| context.memory(|memory| memory.area_rect(id)));
        long
    }

    /// `long_frame` and then `frames` more without input, as the app
    /// draws them: the last.
    fn long_frames(context: &egui::Context, more: usize, events: Vec<egui::Event>, frames: usize) -> Long {
        let mut shown = long_frame(context, more, events);
        for _ in 0..frames {
            shown = long_frame(context, more, Vec::new());
        }
        shown
    }

    /// Opens the long test menu with a click, the pointer left off it.
    fn long_opened(more: usize) -> (egui::Context, Long) {
        let context = themed_context();
        let button = long_frame(&context, more, Vec::new()).button.center();
        long_frame(&context, more, vec![egui::Event::PointerMoved(button), click(button, true)]);
        long_frame(&context, more, vec![click(button, false)]);
        let shown = long_frames(&context, more, vec![egui::Event::PointerMoved(Pos2::new(600.0, 5.0))], 3);
        assert!(shown.menu.is_some(), "the menu opens");
        (context, shown)
    }

    /// Between `top` and the window's bottom.
    fn assert_under(rect: Rect, top: f32, what: &str) {
        assert!(rect.top() >= top - 0.5, "the {what} at {rect:?} is above {top}");
        assert!(rect.bottom() <= SHORT_SCREEN.y + 0.5, "the {what} at {rect:?} runs past the window's bottom at {}", SHORT_SCREEN.y);
    }

    #[test]
    fn a_menu_too_long_for_the_window_scrolls_under_the_title_bar() {
        let (_, shown) = long_opened(5);
        let menu = shown.menu.unwrap();
        assert_under(menu, shown.button.bottom(), "menu");
        let last = shown.rows[LONG_ROWS - 1];
        assert!(last.top() > menu.bottom(), "the last row at {last:?} shows in the menu at {menu:?} before it scrolls");
    }

    #[test]
    fn a_submenu_too_long_for_the_window_opens_beside_its_row_under_the_title_bar() {
        // Near the menu's top, More's submenu runs down from its row; near
        // the window's bottom, up from it.
        for (more, down) in [(2, true), (5, true), (9, false)] {
            let (context, shown) = long_opened(more);
            let menu = shown.menu.unwrap();
            let row = Rect::from_min_size(shown.rows[more - 1].left_bottom(), Vec2::new(menu.width(), MENU_ROW_HEIGHT));
            assert!(row.bottom() < menu.bottom(), "More at {row:?} is out of the menu at {menu:?}");
            let shown = long_frames(&context, more, vec![egui::Event::PointerMoved(row.center())], 3);
            let submenu = shown.submenu.expect("hovering More opens its submenu");
            assert_under(submenu, title_bar::HEIGHT, &format!("submenu of row {more}"));
            assert!(submenu.left() >= menu.right() - 0.5, "{submenu:?} covers {menu:?}");
            let (edge, row_edge) = if down { (submenu.top(), row.top()) } else { (submenu.bottom(), row.bottom()) };
            assert!((edge - row_edge).abs() <= 3.0, "the submenu at {submenu:?} is not level with its row at {row:?}");
            // As tall as there is room for, it scrolls.
            assert!(submenu.height() < LONG_ROWS as f32 * MENU_ROW_HEIGHT);
        }
    }

    #[test]
    fn the_keys_scroll_a_long_menu_to_the_row_they_are_on() {
        let (context, _) = long_opened(5);
        let inside = |shown: &Long, row: usize| {
            let (menu, row) = (shown.menu.unwrap(), shown.rows[row]);
            row.top() >= menu.top() - 0.5 && row.bottom() <= menu.bottom() + 0.5
        };
        let shown = long_frames(&context, 5, vec![key(egui::Key::End, egui::Modifiers::NONE)], 60);
        assert_eq!(shown.focused, Some(LONG_ROWS - 1));
        assert!(inside(&shown, LONG_ROWS - 1), "the last row at {:?} is out of the menu at {:?}", shown.rows[LONG_ROWS - 1], shown.menu);
        assert!(!inside(&shown, 0));
        let shown = long_frames(&context, 5, vec![key(egui::Key::Home, egui::Modifiers::NONE)], 60);
        assert_eq!(shown.focused, Some(0));
        assert!(inside(&shown, 0), "the first row at {:?} is out of the menu at {:?}", shown.rows[0], shown.menu);
    }

    #[test]
    fn the_keys_hold_their_submenu_while_the_pointer_rests_on_another_row() {
        let rows = keyed_frame(&opened(Pos2::new(600.0, 5.0)), Vec::new()).rows;
        let exit = rows.iter().find(|(name, _)| *name == "Exit").unwrap().1.center();
        let context = opened(exit);
        assert_eq!(keyed_frame(&context, Vec::new()).highlighted, ["Exit"]);
        // From the row under the pointer.
        assert_eq!(press(&context, egui::Key::ArrowUp).focused, Some("Recent"));
        let shown = press(&context, egui::Key::ArrowRight);
        assert!(shown.submenu, "the pointer resting on Exit closed Recent");
        assert_eq!(shown.highlighted, ["One", "Recent"]);
        for _ in 0..3 {
            assert!(keyed_frame(&context, Vec::new()).submenu);
        }
        // Moved, the pointer takes over.
        keyed_frame(&context, vec![egui::Event::PointerMoved(exit + Vec2::new(4.0, 0.0))]);
        let moved = keyed_frame(&context, Vec::new());
        assert!(moved.highlighted.contains(&"Exit"), "{:?}", moved.highlighted);
        assert!(!moved.submenu);
    }

    /// How far `inner` sits inside `outer` on its left, right, top and
    /// bottom.
    fn insets(inner: Rect, outer: Rect) -> [f32; 4] {
        [inner.left() - outer.left(), outer.right() - inner.right(), inner.top() - outer.top(), outer.bottom() - inner.bottom()]
    }

    #[test]
    fn a_dialog_keeps_its_contents_off_its_border() {
        let context = themed_context();
        let id = Id::new("test-dialog");
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let mut contents = Rect::NOTHING;
        for _ in 0..3 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let output = context.run_ui(input, |ui| {
                let shown = dialog(ui.ctx(), id).show(ui.ctx(), |ui| {
                    ui.label("Are you sure you want to delete all variables?");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| dialog_button(ui, "No", false));
                    ui.min_rect()
                });
                contents = shown.inner;
            });
            output.drop_without_applying_deltas();
        }
        let window = context.memory(|memory| memory.area_rect(id)).expect("the dialog shows");
        // Not the menus' 1.
        let margin = DIALOG_MARGIN as f32;
        assert!(insets(contents, window).iter().all(|inset| *inset >= margin), "{contents:?} in {window:?}");
    }

    #[test]
    fn a_tooltip_keeps_its_words_off_its_border() {
        let context = themed_context();
        let text = "Bake the selected nodes";
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let frame = |events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
            let mut button = Rect::NOTHING;
            let mut output = context.run_ui(input, |ui| {
                ui.add_space(100.0);
                button = ui.button("Bake").tooltip(text).rect;
            });
            let words = std::mem::take(&mut output.shapes).into_iter().find_map(|clipped| match clipped.shape {
                egui::Shape::Text(shape) if shape.galley.text() == text => Some(shape.galley.rect.translate(shape.pos.to_vec2())),
                _ => None,
            });
            output.drop_without_applying_deltas();
            (button, words)
        };
        let (button, _) = frame(Vec::new());
        frame(vec![egui::Event::PointerMoved(button.center())]);
        for _ in 0..120 {
            let Some(words) = frame(Vec::new()).1 else {
                continue;
            };
            let tooltip = context.memory(|memory| {
                let layers = memory.areas().visible_layer_ids().into_iter().filter(|layer| layer.order == egui::Order::Tooltip);
                layers.filter_map(|layer| memory.area_rect(layer.id)).find(|rect| rect.contains_rect(words))
            });
            let tooltip = tooltip.expect("the words show in a tooltip");
            let [left, right, top, bottom] = insets(words, tooltip);
            let margin = TOOLTIP_MARGIN;
            assert!(left >= margin.left as f32 && right >= margin.right as f32, "{words:?} in {tooltip:?}");
            assert!(top >= margin.top as f32 && bottom >= margin.bottom as f32, "{words:?} in {tooltip:?}");
            return;
        }
        panic!("no tooltip shows");
    }
}
