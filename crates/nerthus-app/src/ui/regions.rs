//! Build Settings' Regions tab (`RegionManager`, with a `RegionControl`
//! per region): the terrain as a square with the build's grid and the
//! previewed node's render behind it, and the regions on it to add, move,
//! resize, rename, recolour and delete.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Color32, CornerRadius, CursorIcon, Id, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};
use nerthus_engine::Region;
use nerthus_export::build::OutputType;

use crate::document::{last_region_position, SWATCHES};
use crate::editor::Editor;
use crate::progress::{ProgressStep, StepEnd};
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::tool_commands::ToolCommand;
use crate::ui::widgets::{check_item, link_button, menu_caption, menu_item, menu_separator, menu_style, submenu, Tooltips};

/// `ProportionateRegionsGridUnit`: a grid cell, in build pixels.
const GRID_UNIT: f32 = 512.0;
/// The smallest side a corner drag leaves a region (`ResizeThumb_DragDelta`).
const SMALLEST_SIDE: f32 = 32.0;
/// How far the mouse goes before a press moves a region.
const DRAG_THRESHOLD: f32 = 5.0;
/// The corner thumbs' side.
const THUMB: f32 = 10.0;
/// `miOpacity*`: the render's opacity behind the regions.
const OPACITIES: [(f32, &str); 4] = [(0.1, "10%"), (0.25, "25%"), (0.5, "50%"), (0.75, "75%")];

/// A corner of a region, as its thumb (`GetResizeDirection`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    const ALL: [Corner; 4] = [Corner::BottomRight, Corner::TopLeft, Corner::TopRight, Corner::BottomLeft];

    /// Which way the region grows from this corner.
    fn direction(self) -> (f32, f32) {
        match self {
            Corner::BottomRight => (1.0, 1.0),
            Corner::TopLeft => (-1.0, -1.0),
            Corner::TopRight => (1.0, -1.0),
            Corner::BottomLeft => (-1.0, 1.0),
        }
    }

    fn thumb(self, rect: Rect) -> Rect {
        let size = Vec2::splat(THUMB);
        let min = match self {
            Corner::TopLeft => rect.min,
            Corner::TopRight => Pos2::new(rect.max.x - THUMB, rect.min.y),
            Corner::BottomLeft => Pos2::new(rect.min.x, rect.max.y - THUMB),
            Corner::BottomRight => rect.max - size,
        };
        Rect::from_min_size(min, size)
    }

    fn cursor(self) -> CursorIcon {
        match self {
            Corner::TopLeft | Corner::BottomRight => CursorIcon::ResizeNwSe,
            Corner::TopRight | Corner::BottomLeft => CursorIcon::ResizeNeSw,
        }
    }
}

/// A press on a region: once the mouse has gone far enough it moves the
/// region (`Canvas_MouseMove`), which takes its new place on release.
#[derive(Clone, Debug)]
struct Move {
    region: String,
    start: Pos2,
    /// The pointer's offset from the region's corner.
    offset: Vec2,
    /// The corner on the canvas while the region moves.
    corner: Option<Vec2>,
}

/// A corner thumb being dragged (`ResizeThumb_DragStarted`); `changed`
/// says a drag delta has moved it (a plain click on the thumb does not
/// end one, `ResizeThumb_DragCompleted`).
#[derive(Clone, Debug)]
struct Resize {
    region: String,
    corner: Corner,
    left: f32,
    top: f32,
    side: f32,
    changed: bool,
}

/// A region being renamed (`EditablePopup`): the text so far, and whether
/// the box has had the keyboard yet.
#[derive(Clone, Debug)]
struct Rename {
    region: String,
    text: String,
    focused: bool,
}

/// What a right click opened its menu on.
#[derive(Clone, Debug)]
enum MenuTarget {
    /// The canvas, at this point (`mousePosition`).
    Canvas(Vec2),
    Region(String),
}

/// The render behind the regions, as a texture, and what it was made
/// from.
struct Background {
    texture: egui::TextureHandle,
    source: BackgroundSource,
}

#[derive(Clone, Debug, PartialEq)]
enum BackgroundSource {
    Render(usize),
    Saved(PathBuf, std::time::SystemTime),
}

/// The tab's state while Build Settings is open.
#[derive(Default)]
pub struct RegionsState {
    /// The region pressed last; its outline shows (`FocusRegion`).
    focused: Option<String>,
    moving: Option<Move>,
    resizing: Option<Resize>,
    menu: Option<MenuTarget>,
    /// Shift-click over regions on top of each other: the ones to choose
    /// from, and the one the next press moves (`nextDragRegion`).
    choices: Vec<String>,
    choices_at: Pos2,
    next_move: Option<String>,
    renaming: Option<Rename>,
    /// `Snap Regions` (for the session).
    snap: bool,
    background: Option<Background>,
}

impl std::fmt::Debug for RegionsState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("RegionsState").field("focused", &self.focused).field("snap", &self.snap).finish_non_exhaustive()
    }
}

impl Clone for RegionsState {
    fn clone(&self) -> Self {
        Self { snap: self.snap, ..Self::default() }
    }
}

/// `MathF.Round(value, 5, MidpointRounding.ToZero)`.
fn truncate_to_five_places(value: f32) -> f32 {
    (value * 100000.0).trunc() / 100000.0
}

/// `MathF.Round(value, 5)` (ties to even).
fn round_to_five_places(value: f32) -> f32 {
    (value * 100000.0).round_ties_even() / 100000.0
}

/// The region's square on the canvas.
fn region_rect(region: &Region, canvas: Rect) -> Rect {
    let width = canvas.width();
    let corner = canvas.min + Vec2::new(region.position[0] * width, region.position[1] * width);
    let side = region.width * width;
    Rect::from_min_size(corner, Vec2::splat(side))
}

/// `RegionControl.UpdateLabels`: `Size (ProportionateWidth)` over
/// `ProportionatePosition` (`Float2.ToString`).
fn data_text(region: &Region) -> String {
    format!("{} ({:.2}) \n{:.3} x {:.3}", region.size, region.width, region.position[0], region.position[1])
}

/// Where Shell's "Save Current Node preview" keeps its picture:
/// `<project file>.jpg` (holding PNG data).
pub fn reference_file(project: &Path) -> PathBuf {
    let mut name = project.as_os_str().to_os_string();
    name.push(".jpg");
    PathBuf::from(name)
}

/// The tab: the "Manage Regions" bar with the region preview menu, the
/// resolution warning and the canvas.
pub fn regions_tab(ui: &mut Ui, editor: &mut Editor, state: &mut RegionsState, commands: &mut Vec<Command>) {
    header(ui, editor, commands);
    let definition = editor.document.build_definition();
    // `CheckRegionWarning`.
    if definition.resolution <= 2048 {
        ui.add_space(8.0);
        warning(ui, "Regions require the base resolution of the terrain to be at least 4096.");
    }
    ui.add_space(8.0);
    canvas(ui, editor, state, commands);
}

/// A red notice on `Panel-Border`, centred.
pub fn warning(ui: &mut Ui, text: &str) {
    let width = ui.available_width();
    let galley = ui.painter().layout(text.to_string(), theme::interface(12.0), theme::ACCENT_RED, width - 24.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, galley.size().y + 8.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let position = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.top() + 4.0);
    ui.painter().galley(position, galley, theme::ACCENT_RED);
}

/// "Manage Regions", then its Help (the documentation on regions) and
/// "Preview region" with its menu (`miBuildSettingsRegionsMenu`).
fn header(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PANEL_BORDER);
    let title = ui.painter().layout_no_wrap("Manage Regions".to_string(), theme::semibold(12.0), theme::LABEL_EMPHASIS);
    ui.painter().galley(Pos2::new(rect.left() + 12.0, rect.center().y - title.size().y / 2.0), title, theme::LABEL_EMPHASIS);

    let mut bar = ui.new_child(egui::UiBuilder::new().max_rect(rect.shrink2(Vec2::new(8.0, 2.0))).layout(egui::Layout::right_to_left(egui::Align::Center)));
    if link_button(&mut bar, "Help", theme::interface(12.0), Vec2::ZERO).clicked() {
        commands.push(Command::Tools(ToolCommand::ShowDocs(crate::docs::REGIONS.to_string())));
    }
    bar.add_space(8.0);
    let current = editor.preview_region.clone().unwrap_or_else(|| "Whole Terrain".to_string());
    let response = bar.button(current);
    let mut popup = egui::Popup::menu(&response);
    popup = popup.close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
    popup.show(|ui| crate::ui::widgets::menu_rows_below(ui, response.interact_rect, |ui| regions_menu(ui, editor, commands, false)));
    bar.label(egui::RichText::new("Preview region").font(theme::interface(12.0)).color(theme::LABEL_DEFAULT));
}

/// `Shell.PopulateRegionsMenu`: Whole Terrain; then, with a node
/// previewed, the regions, "Save Current Node preview" and (when asked)
/// "Edit Regions".
#[cfg(target_os = "macos")]
pub fn regions_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>, edit_regions: bool) {
    let entries = crate::ui::main_menu::region_entries(editor, edit_regions);
    crate::ui::main_menu::show_entries(ui, &entries, commands);
}

#[cfg(not(target_os = "macos"))]
pub fn regions_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>, edit_regions: bool) {
    menu_style(ui);
    let previewed = editor.preview_region.as_deref();
    if check_item(ui, previewed.is_none(), "Whole Terrain", "").clicked() {
        commands.push(Command::PreviewRegion(None));
        ui.close();
    }
    menu_separator(ui);
    menu_caption(ui, "Regions");
    if editor.preview_node.is_none() {
        return;
    }
    for region in editor.document.regions() {
        let checked = previewed == Some(region.name.as_str());
        if check_item(ui, checked, &region.name, "").clicked() {
            commands.push(Command::PreviewRegion(Some(region.name.clone())));
            ui.close();
        }
    }
    menu_separator(ui);
    if menu_item(ui, None, None, "Save Current Node preview", "").clicked() {
        commands.push(Command::SaveRegionReference);
        ui.close();
    }
    if edit_regions && menu_item(ui, None, None, "Edit Regions", "").clicked() {
        commands.push(Command::EditRegions);
        ui.close();
    }
}

/// The square: render, grids, regions, and what the mouse does to them.
fn canvas(ui: &mut Ui, editor: &mut Editor, state: &mut RegionsState, commands: &mut Vec<Command>) {
    let side = ui.available_width().min(ui.available_height().max(320.0)).max(160.0);
    let (outer, response) = ui.allocate_exact_size(Vec2::splat(side), Sense::click_and_drag());
    let canvas = outer.shrink(1.5);
    let painter = ui.painter_at(outer);

    // `BackgroundBorder`: `Popup-Border`, or the render at its opacity.
    painter.rect_filled(outer, 0.0, theme::POPUP_BORDER);
    show_loading_preview(editor);
    let opacity = editor.user.options.region_reference_opacity();
    if let Some(texture) = background_texture(ui.ctx(), editor, state) {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        let tint = Color32::from_white_alpha((opacity * 255.0 + 0.5) as u8);
        painter.image(texture.id(), canvas, uv, tint);
    }
    painter.rect_stroke(outer, 0.0, Stroke::new(1.5, theme::BUTTON_BORDER), StrokeKind::Inside);

    // `CalculateStats`: the grid's cells are 512 build pixels; the tiles'
    // grid shows (at 0.3) with tiled output.
    let definition = editor.document.build_definition();
    let world = definition.world_resolution().max(1) as f32;
    let cells = ((world / GRID_UNIT) * (world / GRID_UNIT)) as i64;
    draw_grid(&painter, canvas, (cells as f64).sqrt() as usize, Color32::from_rgb(0x11, 0x11, 0x11));
    if definition.output_type != OutputType::SingleImage {
        let tiles = definition.number_of_tiles;
        let faded = Color32::from_rgba_unmultiplied(0x11, 0x11, 0x11, 77);
        draw_grid(&painter, canvas, tiles, faded);
    }
    // What snapping rounds to, a grid cell on the canvas.
    let snap_unit = canvas.width() * (GRID_UNIT / world);

    let mut regions = editor.document.regions();
    let pointer = response.interact_pointer_pos().or_else(|| ui.ctx().pointer_hover_pos());

    // `Canvas_MouseDown`: choose the region as soon as the button goes
    // down, before egui decides whether this is a click or a drag.
    let primary_pressed = ui.input(|input| input.pointer.button_pressed(egui::PointerButton::Primary));
    let pressed_on_canvas = response.is_pointer_button_down_on() || response.clicked_by(egui::PointerButton::Primary);
    if primary_pressed && pressed_on_canvas {
        let origin = ui.input(|input| input.pointer.press_origin()).or(pointer);
        if let Some(point) = origin {
            press(ui, state, canvas, point, &regions);
        }
    }

    // A move in progress follows the mouse; a resize changes the region
    // as it goes (`ResizeThumb_DragDelta`).
    if response.dragged()
        && let Some(point) = pointer
    {
        drag_moving(state, canvas, point, snap_unit, &regions);
        if let Some(changed) = drag_resizing(state, canvas, point, snap_unit, &regions) {
            let name = changed.name.clone();
            editor.document.update_region(&name, changed);
            regions = editor.document.regions();
        }
    }
    let primary_released = ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
    if primary_released || response.drag_stopped() {
        finish_drag(editor, state, canvas);
        regions = editor.document.regions();
    }

    for region in &regions {
        let mut rect = region_rect(region, canvas);
        if let Some(moving) = &state.moving
            && moving.region == region.name
            && let Some(corner) = moving.corner
        {
            rect = Rect::from_min_size(canvas.min + corner, rect.size());
        }
        let focused = state.focused.as_deref() == Some(region.name.as_str());
        let previewed = editor.preview_region.as_deref() == Some(region.name.as_str());
        draw_region(&painter, region, rect, focused, previewed);
    }

    hover_cursor(ui, state, canvas, pointer, &regions);

    if response.secondary_clicked()
        && let Some(point) = pointer
    {
        let target = match topmost(&regions, canvas, point) {
            Some(region) => MenuTarget::Region(region.name.clone()),
            None => MenuTarget::Canvas(point - canvas.min),
        };
        state.menu = Some(target);
    }
    let menu_target = state.menu.clone();
    crate::ui::widgets::context_menu(&response, |ui| match &menu_target {
        Some(MenuTarget::Region(name)) => region_menu(ui, editor, state, name, commands),
        _ => canvas_menu(ui, editor, state, canvas, commands),
    });

    choices_popup(ui, state);
    rename_popup(ui, editor, state, canvas, commands);
}

/// `GridControl`: `cells` cells a side, 1-pixel lines.
fn draw_grid(painter: &egui::Painter, canvas: Rect, cells: usize, color: Color32) {
    if cells == 0 {
        return;
    }
    let step = canvas.width() / cells as f32;
    let stroke = Stroke::new(1.0, color);
    for index in 0..=cells {
        let offset = index as f32 * step;
        painter.hline(canvas.x_range(), canvas.top() + offset, stroke);
        painter.vline(canvas.left() + offset, canvas.y_range(), stroke);
    }
}

/// `RegionControl`: the region's colour at 35% under a 30% black shade,
/// a 2-pixel outline in its colour, its name (on its colour when it is the
/// one previewed, `SelectRegion`), its size and place in the middle, the
/// focus outline (`FocusRegion`) and the four corner thumbs. The labels
/// are cut off at their places in the region however small it is: the
/// name (`txtRegionName`: margin 1, padding 8,2, top left) and the size and
/// place (`txtData`: padding 8,2, its lines centred in the middle).
fn draw_region(painter: &egui::Painter, region: &Region, rect: Rect, focused: bool, previewed: bool) {
    let color = theme::from_sc_rgb(region.color, 1.0);
    let fill = theme::from_sc_rgb(region.color, 0.35);
    painter.rect_filled(rect, 0.0, fill);
    painter.rect_filled(rect, 0.0, Color32::from_black_alpha(77));
    painter.rect_stroke(rect, 0.0, Stroke::new(2.0, color), StrokeKind::Inside);
    if focused {
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, theme::ACCENT_PRIMARY), StrokeKind::Inside);
    }

    let name_color = if previewed { theme::WINDOW_DARK } else { color };
    let name_font = if previewed { theme::monospace(12.0) } else { theme::monospace_bold(12.0) };
    let name_slot = rect.shrink(1.0);
    let name_painter = painter.with_clip_rect(painter.clip_rect().intersect(name_slot));
    let name = name_painter.layout_no_wrap(region.name.clone(), name_font, name_color);
    let name_rect = Rect::from_min_size(name_slot.min, name.size() + Vec2::new(16.0, 4.0));
    if previewed {
        name_painter.rect_filled(name_rect, 0.0, color);
    }
    name_painter.galley(name_rect.min + Vec2::new(8.0, 2.0), name, name_color);

    // Each line centred without its trailing space, as WPF aligns them.
    let data_painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    let data_text = data_text(region);
    let lines: Vec<_> = data_text.lines().map(|line| data_painter.layout_no_wrap(line.trim_end().to_string(), theme::monospace(12.0), color)).collect();
    let data_height: f32 = lines.iter().map(|line| line.size().y).sum();
    let mut line_top = rect.center().y - data_height / 2.0;
    for line in lines {
        let line_height = line.size().y;
        let line_left = rect.center().x - line.size().x / 2.0;
        data_painter.galley(Pos2::new(line_left, line_top), line, color);
        line_top += line_height;
    }

    for corner in Corner::ALL {
        painter.rect_filled(corner.thumb(rect), CornerRadius::ZERO, Color32::from_rgb(0x80, 0x80, 0x80));
    }
}

/// The regions under `point`, topmost (last drawn) first.
fn under(regions: &[Region], canvas: Rect, point: Pos2) -> Vec<&Region> {
    let mut found: Vec<&Region> = regions.iter().filter(|region| region_rect(region, canvas).contains(point)).collect();
    found.reverse();
    found
}

fn topmost(regions: &[Region], canvas: Rect, point: Pos2) -> Option<&Region> {
    under(regions, canvas, point).into_iter().next()
}

/// The corner thumb under `point`, topmost region first.
fn thumb_at(regions: &[Region], canvas: Rect, point: Pos2) -> Option<(String, Corner)> {
    for region in regions.iter().rev() {
        let rect = region_rect(region, canvas);
        for corner in Corner::ALL {
            if corner.thumb(rect).contains(point) {
                return Some((region.name.clone(), corner));
            }
        }
    }
    None
}

/// `Canvas_MouseDown` and `ResizeThumb_DragStarted`.
fn press(ui: &Ui, state: &mut RegionsState, canvas: Rect, point: Pos2, regions: &[Region]) {
    state.focused = None;
    if let Some((name, corner)) = thumb_at(regions, canvas, point)
        && let Some(region) = regions.iter().find(|region| region.name == name)
    {
        let rect = region_rect(region, canvas);
        let left = rect.left() - canvas.left();
        let top = rect.top() - canvas.top();
        state.resizing = Some(Resize { region: name.clone(), corner, left, top, side: rect.width(), changed: false });
        state.focused = Some(name);
        return;
    }
    if let Some(next) = state.next_move.take()
        && let Some(region) = regions.iter().find(|region| region.name == next)
        && region_rect(region, canvas).contains(point)
    {
        start_move(state, region, canvas, point);
        return;
    }
    let found = under(regions, canvas, point);
    if found.is_empty() {
        return;
    }
    let modifiers = ui.input(|input| input.modifiers);
    if modifiers.shift && found.len() > 1 {
        state.choices = found.iter().map(|region| region.name.clone()).collect();
        state.choices_at = point;
        return;
    }
    let chosen = if modifiers.alt && found.len() > 1 { found[1] } else { found[0] };
    start_move(state, chosen, canvas, point);
}

fn start_move(state: &mut RegionsState, region: &Region, canvas: Rect, point: Pos2) {
    let rect = region_rect(region, canvas);
    state.moving = Some(Move { region: region.name.clone(), start: point, offset: point - rect.min, corner: None });
    state.focused = Some(region.name.clone());
}

/// `RegionManager.Canvas_MouseMove`: past the threshold the region
/// follows the mouse, snapped if asked, kept on the canvas.
fn drag_moving(state: &mut RegionsState, canvas: Rect, point: Pos2, snap_unit: f32, regions: &[Region]) {
    let snap = state.snap;
    let Some(moving) = state.moving.as_mut() else {
        return;
    };
    let Some(region) = regions.iter().find(|region| region.name == moving.region) else {
        return;
    };
    let rect = region_rect(region, canvas);
    if moving.corner.is_none() {
        let travelled = point - moving.start;
        if travelled.x.abs() <= DRAG_THRESHOLD && travelled.y.abs() <= DRAG_THRESHOLD {
            return;
        }
        let corner = rect.min - canvas.min;
        moving.offset = point - rect.min;
        moving.corner = Some(corner);
    }
    let mut corner = point - canvas.min - moving.offset;
    if snap && snap_unit > 0.0 {
        corner.x = (corner.x / snap_unit).trunc() * snap_unit;
        corner.y = (corner.y / snap_unit).trunc() * snap_unit;
    }
    // `EnsureWithinBounds` (the canvas is as tall as it is wide).
    let bound = canvas.width();
    let side = rect.width();
    if corner.x < 0.0 {
        corner.x = 0.0;
    }
    if corner.x + side > bound {
        corner.x = bound - side;
    }
    if corner.y < 0.0 {
        corner.y = 0.0;
    }
    if corner.y + side > bound {
        corner.y = bound - side;
    }
    moving.corner = Some(corner);
}

/// `ResizeThumb_DragDelta`: the region stays square, grown from the
/// corner opposite the thumb and kept on the canvas. The minimum is 32
/// pixels (whole grid cells when snapped), limited by the available room.
/// Its place and width change as it goes. Returns the region changed.
fn drag_resizing(state: &mut RegionsState, canvas: Rect, point: Pos2, snap_unit: f32, regions: &[Region]) -> Option<Region> {
    let resizing = state.resizing.as_mut()?;
    let region = regions.iter().find(|region| region.name == resizing.region)?;
    let (x_direction, y_direction) = resizing.corner.direction();
    let anchor_x = if x_direction < 0.0 { resizing.left + resizing.side } else { resizing.left };
    let anchor_y = if y_direction < 0.0 { resizing.top + resizing.side } else { resizing.top };
    let mouse = point - canvas.min;
    let reach = (mouse.x - anchor_x).abs().max((mouse.y - anchor_y).abs());
    let bound = canvas.width();
    let available_x = if x_direction < 0.0 { anchor_x } else { bound - anchor_x };
    let available_y = if y_direction < 0.0 { anchor_y } else { bound - anchor_y };
    let largest_side = available_x.min(available_y);
    if bound <= 0.0 || largest_side <= 0.0 {
        return None;
    }
    let mut side = reach.max(SMALLEST_SIDE).min(largest_side);
    if state.snap && snap_unit > 0.0 {
        let largest_snapped_side = (largest_side / snap_unit).trunc() * snap_unit;
        if largest_snapped_side > 0.0 {
            // The side rounds to the nearest whole cell (a side 1.9 cells
            // long stays 2), so a side under one cell never truncates to 0.
            let smallest_snapped_side = (SMALLEST_SIDE / snap_unit).ceil().max(1.0) * snap_unit;
            let snapped_side = (side / snap_unit).round() * snap_unit;
            side = snapped_side.max(smallest_snapped_side).min(largest_snapped_side);
        }
    }
    // Derive the dragged corner only after snapping and bounds have
    // settled the side, so the opposite corner remains the anchor.
    let left = if x_direction < 0.0 { anchor_x - side } else { anchor_x };
    let top = if y_direction < 0.0 { anchor_y - side } else { anchor_y };
    let mut changed = region.clone();
    changed.width = side / bound;
    let last = last_region_position(changed.width);
    changed.position = [truncate_to_five_places(left / bound).clamp(0.0, last), round_to_five_places(top / bound).clamp(0.0, last)];
    resizing.changed = true;
    Some(changed)
}

/// `Canvas_MouseUp` (a moved region takes its place, rounded to five
/// places from its whole-pixel corner and kept wholly on the terrain) and
/// `ResizeThumb_DragCompleted`; either way a previewed region is cut again
/// (`RegionUpdated`).
fn finish_drag(editor: &mut Editor, state: &mut RegionsState, canvas: Rect) {
    if let Some(resizing) = state.resizing.take()
        && resizing.changed
    {
        editor.region_changed(&resizing.region);
    }
    let Some(moving) = state.moving.take() else {
        return;
    };
    let Some(corner) = moving.corner else {
        return;
    };
    let Some(mut region) = editor.document.regions().into_iter().find(|region| region.name == moving.region) else {
        return;
    };
    let bound = canvas.width();
    let left = corner.x.trunc();
    let top = corner.y.trunc();
    let last = last_region_position(region.width);
    region.position = [truncate_to_five_places(left / bound).clamp(0.0, last), round_to_five_places(top / bound).clamp(0.0, last)];
    editor.document.update_region(&moving.region, region);
    editor.region_changed(&moving.region);
}

fn hover_cursor(ui: &Ui, state: &RegionsState, canvas: Rect, pointer: Option<Pos2>, regions: &[Region]) {
    if let Some(resizing) = &state.resizing {
        ui.ctx().set_cursor_icon(resizing.corner.cursor());
        return;
    }
    if state.moving.as_ref().is_some_and(|moving| moving.corner.is_some()) {
        ui.ctx().set_cursor_icon(CursorIcon::Move);
        return;
    }
    let Some(point) = pointer.filter(|point| canvas.contains(*point)) else {
        return;
    };
    if let Some((_, corner)) = thumb_at(regions, canvas, point) {
        ui.ctx().set_cursor_icon(corner.cursor());
    }
}

/// `RegionControl`'s menu: Rename..., Set Color (the swatches) and
/// Delete this Region.
fn region_menu(ui: &mut Ui, editor: &mut Editor, state: &mut RegionsState, name: &str, commands: &mut Vec<Command>) {
    menu_style(ui);
    if menu_item(ui, None, None, "Rename...", "").clicked() {
        state.renaming = Some(Rename { region: name.to_string(), text: name.to_string(), focused: false });
        ui.close();
    }
    menu_caption(ui, "Color");
    submenu(ui, None, None, "Set Color", |ui| {
        let mut chosen = None;
        egui::Grid::new("region-swatches").spacing(Vec2::splat(4.0)).show(ui, |ui| {
            for (index, swatch) in SWATCHES.iter().enumerate() {
                let color = theme::swatch_color(swatch);
                let (rect, response) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                ui.painter().rect_filled(rect, 2.0, color);
                if response.tooltip(*swatch).clicked() {
                    chosen = Some(color);
                }
                if index == SWATCHES.len() / 2 - 1 {
                    ui.end_row();
                }
            }
        });
        if let Some(color) = chosen {
            if let Some(mut region) = editor.document.regions().into_iter().find(|region| region.name == name) {
                region.color = theme::sc_rgb(color);
                editor.document.update_region(name, region);
            }
            ui.close();
        }
    });
    if menu_item(ui, None, None, "Delete this Region", "").clicked() {
        commands.push(Command::DeleteRegion(name.to_string()));
        ui.close();
    }
}

/// The canvas's menu: Add Region (at the click), Show Saved Reference
/// (when there is one), the render's opacity and Snap Regions.
fn canvas_menu(ui: &mut Ui, editor: &mut Editor, state: &mut RegionsState, canvas: Rect, commands: &mut Vec<Command>) {
    menu_style(ui);
    let point = match &state.menu {
        Some(MenuTarget::Canvas(point)) => *point,
        _ => Vec2::ZERO,
    };
    if menu_item(ui, Some("plus"), Some(theme::ACCENT_GREEN), "Add Region", "").clicked() {
        // `AddNewRegion`: the click over the manager's width.
        let corner = [point.x / canvas.width(), point.y / canvas.width()];
        commands.push(Command::AddRegion(corner));
        ui.close();
    }
    let saved = editor.document.path.as_deref().map(reference_file).filter(|file| file.exists());
    if saved.is_some() {
        let showing = editor.user.options.show_region_reference();
        if check_item(ui, showing, "Show Saved Reference", "").clicked() {
            editor.user.options.set_show_region_reference(!showing);
            state.background = None;
        }
    }
    menu_separator(ui);
    menu_caption(ui, "Preview image opacity");
    let current = editor.user.options.region_reference_opacity();
    for (opacity, label) in OPACITIES {
        let checked = (opacity - current).abs() < 1e-5;
        if check_item(ui, checked, label, "").clicked() {
            editor.user.options.set_region_reference_opacity(opacity);
        }
    }
    menu_separator(ui);
    let snap_item = check_item(ui, state.snap, "Snap Regions", "");
    let snap_item = snap_item.tooltip("Enables snapping behavior for regions on the canvas during drag or resize actions.");
    if snap_item.clicked() {
        state.snap = !state.snap;
    }
}

/// Shift-click over regions on top of each other: pick the one the next
/// press moves.
fn choices_popup(ui: &Ui, state: &mut RegionsState) {
    if state.choices.is_empty() {
        return;
    }
    let mut chosen = None;
    let mut area = egui::Area::new(Id::new("region-choices"));
    area = area.fixed_pos(state.choices_at);
    area = area.order(egui::Order::Foreground);
    let response = area.show(ui.ctx(), |ui| {
        egui::Frame::menu(ui.style()).show(ui, |ui| {
            menu_style(ui);
            for name in &state.choices {
                if menu_item(ui, None, None, name, "").clicked() {
                    chosen = Some(name.clone());
                }
            }
        });
    });
    if let Some(name) = chosen {
        state.focused = Some(name.clone());
        state.next_move = Some(name);
        state.choices.clear();
        return;
    }
    if response.response.clicked_elsewhere() {
        state.choices.clear();
    }
}

/// `EditablePopup` under the region's name, closing as
/// [`crate::ui::widgets::popup_outcome`] says: taken, the name renames the
/// region (trimmed; blank or unchanged does nothing, a name another region
/// has, in any case, is refused).
fn rename_popup(ui: &Ui, editor: &mut Editor, state: &mut RegionsState, canvas: Rect, commands: &mut Vec<Command>) {
    let Some(rename) = state.renaming.as_mut() else {
        return;
    };
    // A duplicate-name message keeps the retry until its modal is closed.
    if ui.memory(|memory| memory.top_modal_layer().is_some()) {
        rename.focused = false;
        return;
    }
    let Some(region) = editor.document.regions().into_iter().find(|region| region.name == rename.region) else {
        state.renaming = None;
        return;
    };
    let rect = region_rect(&region, canvas);
    let mut area = egui::Area::new(Id::new("region-rename"));
    area = area.fixed_pos(rect.min + Vec2::new(1.0, 24.0));
    area = area.order(egui::Order::Foreground);
    let shown = area.show(ui.ctx(), |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            let mut field = egui::TextEdit::singleline(&mut rename.text);
            field = field.font(theme::monospace(16.0));
            field = field.desired_width(160.0);
            ui.add(field)
        }).inner
    });
    let response = shown.inner;
    if !rename.focused {
        response.request_focus();
        let length = rename.text.chars().count();
        let mut text_state = egui::TextEdit::load_state(ui.ctx(), response.id).unwrap_or_default();
        let selection = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(length));
        text_state.cursor.set_char_range(Some(selection));
        text_state.store(ui.ctx(), response.id);
        rename.focused = true;
        // Opening the box (or retrying) must not accept its initiating click.
        return;
    }
    let mut outcome = crate::ui::widgets::popup_outcome(ui, &response);
    // A click elsewhere closes it as leaving the box does.
    if outcome.is_none() && shown.response.clicked_elsewhere() {
        outcome = Some(theme::settings(ui.ctx()).accept_on_lost_focus);
    }
    let Some(submitted) = outcome else {
        return;
    };
    if !submitted {
        state.renaming = None;
        return;
    }
    let new_name = rename.text.trim().to_string();
    if new_name.is_empty() || new_name == region.name {
        state.renaming = None;
        return;
    }
    let taken = editor.document.regions().iter().any(|other| other.name != region.name && other.name.to_lowercase() == new_name.to_lowercase());
    if taken {
        // The box opens again with the name typed.
        commands.push(Command::RegionNameTaken);
        rename.focused = false;
        return;
    }
    let old_name = region.name.clone();
    let mut renamed = region;
    renamed.name = new_name.clone();
    editor.document.update_region(&old_name, renamed);
    if editor.preview_region.as_deref() == Some(old_name.as_str()) {
        editor.preview_region = Some(new_name);
    }
    state.renaming = None;
}

/// `ShowCurrentNodePreview`'s progress window: "Loading preview" while the
/// render behind the regions (not the saved reference) is still on its
/// way for the selected node. The editor ends it once the render is in.
fn show_loading_preview(editor: &mut Editor) {
    let saved = editor.document.path.as_deref().map(reference_file).is_some_and(|file| file.exists());
    let shows_saved = saved && editor.user.options.show_region_reference();
    let rendering = !shows_saved && editor.active_node().is_some() && !editor.building && !editor.render_settled();
    if !rendering || editor.progress.is_some() {
        return;
    }
    let text = "Loading preview, please wait this may take a while...";
    let step = ProgressStep::with_text("Loading preview", text, StepEnd::RegionPreview);
    editor.show_progress(vec![step]);
}

/// The render behind the regions: the saved reference when it is asked
/// for and there, else the previewed node's render (`ShowReferenceImage`).
fn background_texture(ctx: &egui::Context, editor: &Editor, state: &mut RegionsState) -> Option<egui::TextureHandle> {
    let saved = editor.document.path.as_deref().map(reference_file).filter(|file| file.exists());
    let wanted = match saved.filter(|_| editor.user.options.show_region_reference()) {
        Some(file) => {
            let modified = std::fs::metadata(&file).and_then(|metadata| metadata.modified()).ok()?;
            BackgroundSource::Saved(file, modified)
        }
        None => {
            let colours = editor.render_colours.as_ref()?;
            BackgroundSource::Render(Arc::as_ptr(colours) as usize)
        }
    };
    if let Some(background) = &state.background
        && background.source == wanted
    {
        return Some(background.texture.clone());
    }
    let image = match &wanted {
        BackgroundSource::Saved(file, _) => load_png(file)?,
        BackgroundSource::Render(_) => {
            let colours = editor.render_colours.as_ref()?;
            egui::ColorImage::from_rgb([colours.width, colours.height], &colours.rgb)
        }
    };
    let texture = ctx.load_texture("region-reference", image, egui::TextureOptions::LINEAR);
    state.background = Some(Background { texture: texture.clone(), source: wanted });
    Some(texture)
}

fn load_png(file: &Path) -> Option<egui::ColorImage> {
    let bytes = std::fs::read(file).ok()?;
    let pixmap = resvg::tiny_skia::Pixmap::decode_png(&bytes).ok()?;
    let size = [pixmap.width() as usize, pixmap.height() as usize];
    Some(egui::ColorImage::from_rgba_premultiplied(size, pixmap.data()))
}

/// Shell's `SaveCurrentNodePreview`: the previewed node's render written
/// as PNG to `<project file>.jpg`.
pub fn save_reference(editor: &mut Editor) {
    let Some(project) = editor.document.path.clone() else {
        editor.report("Save the project first.");
        return;
    };
    let Some(colours) = editor.render_colours.clone() else {
        return;
    };
    let mut rgba = Vec::with_capacity(colours.width * colours.height * 4);
    let (pixels, _) = colours.rgb.as_chunks::<3>();
    for pixel in pixels {
        rgba.extend_from_slice(pixel);
        rgba.push(255);
    }
    let size = resvg::tiny_skia::IntSize::from_wh(colours.width as u32, colours.height as u32);
    let pixmap = size.and_then(|size| resvg::tiny_skia::Pixmap::from_vec(rgba, size));
    let encoded = pixmap.and_then(|pixmap| pixmap.encode_png().ok());
    let file = reference_file(&project);
    match encoded.map(|bytes| std::fs::write(&file, bytes)) {
        Some(Ok(())) => editor.report(format!("Saved {}", file.display())),
        Some(Err(error)) => editor.report(format!("Cannot save {}: {error}", file.display())),
        None => editor.report("Cannot encode the preview."),
    }
}

/// The region preview's icon menu's tooltip (`miRegionsPreview`).
pub fn preview_tooltip(editor: &Editor) -> String {
    let current = editor.preview_region.as_deref().unwrap_or("Whole Terrain");
    format!("Current region : {current}")
}

/// Whether the region preview menus show (`RegionPreviewMenuVisibility`):
/// once the terrain has a region.
pub fn menus_visible(editor: &Editor) -> bool {
    !editor.document.regions().is_empty()
}

#[cfg(test)]
#[path = "regions-rename-tests.rs"]
mod rename_tests;

#[cfg(test)]
mod resize_tests {
    use super::*;

    const CANVAS_SIDE: f32 = 512.0;

    fn resize(region: &Region, corner: Corner, mouse: Vec2, snap_unit: Option<f32>) -> Region {
        let canvas = Rect::from_min_size(Pos2::new(17.0, 23.0), Vec2::splat(CANVAS_SIDE));
        let resizing = Resize {
            region: region.name.clone(),
            corner,
            left: region.position[0] * CANVAS_SIDE,
            top: region.position[1] * CANVAS_SIDE,
            side: region.width * CANVAS_SIDE,
            changed: false,
        };
        let mut state = RegionsState { resizing: Some(resizing), snap: snap_unit.is_some(), ..RegionsState::default() };
        let regions = [region.clone()];
        let point = canvas.min + mouse;
        drag_resizing(&mut state, canvas, point, snap_unit.unwrap_or(0.0), &regions).expect("the region can be resized")
    }

    fn opposite_corner(region: &Region, corner: Corner) -> Vec2 {
        let (x_direction, y_direction) = corner.direction();
        let left = region.position[0] * CANVAS_SIDE;
        let top = region.position[1] * CANVAS_SIDE;
        let side = region.width * CANVAS_SIDE;
        let x = if x_direction < 0.0 { left + side } else { left };
        let y = if y_direction < 0.0 { top + side } else { top };
        Vec2::new(x, y)
    }

    #[test]
    fn snapped_shrink_keeps_a_positive_minimum_and_the_opposite_corner() {
        let mut region = Region::new("Resize");
        region.position = [0.25, 0.375];
        region.width = 0.25;
        for corner in Corner::ALL {
            let anchor = opposite_corner(&region, corner);
            for (snap_unit, expected_side) in [(64.0, 64.0), (16.0, 32.0), (12.0, 36.0)] {
                let changed = resize(&region, corner, anchor, Some(snap_unit));
                assert_eq!(changed.width * CANVAS_SIDE, expected_side, "{corner:?}, {snap_unit}");
                let final_anchor = opposite_corner(&changed, corner);
                assert!((final_anchor - anchor).length() < 0.01, "{corner:?}, {snap_unit}: {final_anchor:?} moved from {anchor:?}");
            }
        }
    }

    #[test]
    fn snapped_growth_keeps_every_corner_anchored_and_inside_the_canvas() {
        let mut region = Region::new("Resize");
        region.position = [0.25, 0.375];
        region.width = 0.25;
        for corner in Corner::ALL {
            let anchor = opposite_corner(&region, corner);
            let (x_direction, y_direction) = corner.direction();
            let mouse = anchor + Vec2::new(x_direction * 1000.0, y_direction * 1000.0);
            let changed = resize(&region, corner, mouse, Some(60.0));
            assert_eq!((changed.width * CANVAS_SIDE) % 60.0, 0.0, "{corner:?}");
            for position in changed.position {
                assert!(position >= 0.0 && position + changed.width <= 1.0, "{corner:?}: {changed:?}");
            }
            let final_anchor = opposite_corner(&changed, corner);
            assert!((final_anchor - anchor).length() < 0.01, "{corner:?}: {final_anchor:?} moved from {anchor:?}");
        }
    }

    #[test]
    fn unsnapped_resize_limits_both_axes_before_placing_the_dragged_corner() {
        let mut region = Region::new("Resize");
        region.position = [0.25, 0.375];
        region.width = 0.25;
        let anchor = opposite_corner(&region, Corner::TopLeft);
        let changed = resize(&region, Corner::TopLeft, Vec2::splat(-1000.0), None);
        assert_eq!(changed.position, [0.0, 0.125]);
        assert_eq!(changed.width, 0.5);
        assert_eq!(opposite_corner(&changed, Corner::TopLeft), anchor);
    }

    #[test]
    fn small_anchored_regions_stay_positive_and_in_bounds_when_a_grid_cell_cannot_fit() {
        let mut region = Region::new("Resize");
        region.width = 16.0 / CANVAS_SIDE;
        let anchor = opposite_corner(&region, Corner::TopLeft);
        for snap_unit in [None, Some(64.0)] {
            let changed = resize(&region, Corner::TopLeft, Vec2::splat(-1000.0), snap_unit);
            assert_eq!(changed.position, [0.0, 0.0]);
            assert_eq!(changed.width, region.width);
            assert_eq!(opposite_corner(&changed, Corner::TopLeft), anchor);
        }
    }

    #[test]
    fn rounded_resize_position_keeps_the_region_within_the_terrain() {
        let mut region = Region::new("Resize");
        region.position = [0.0, 0.123457];
        region.width = 0.25;
        let changed = resize(&region, Corner::BottomRight, Vec2::splat(1000.0), None);
        assert!(changed.width > 0.0);
        for position in changed.position {
            assert!(position >= 0.0 && position + changed.width <= 1.0, "{changed:?}");
        }
    }

    #[test]
    fn snapped_sides_round_to_the_nearest_cell_and_never_below_one() {
        let mut region = Region::new("Resize");
        region.position = [0.0, 0.0];
        region.width = 0.5;
        // A side 1.9 cells across rounds up to 2 cells.
        let changed = resize(&region, Corner::BottomRight, Vec2::splat(190.0), Some(100.0));
        assert_eq!(changed.width * CANVAS_SIDE, 200.0);
        assert_eq!(changed.position, [0.0, 0.0]);
        // 0.6 cells stops at one cell instead of truncating to 0.
        let changed = resize(&region, Corner::BottomRight, Vec2::splat(60.0), Some(100.0));
        assert_eq!(changed.width * CANVAS_SIDE, 100.0);
        assert_eq!(changed.position, [0.0, 0.0]);
    }
}

#[cfg(test)]
#[path = "regions-selection-tests.rs"]
mod selection_tests;
