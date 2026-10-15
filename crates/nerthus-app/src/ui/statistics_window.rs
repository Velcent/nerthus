//! `StatisticsWindow` ("Statistics", 1200 by 720): two `DonutChart`s on
//! the left, each node's build time and the memory its cache takes, and
//! on the right every node with both and how long it took (`hh:mm:ss.fff`),
//! slowest first, under a TOTAL row. Clicking a slice chooses its node in
//! the list; double-clicking a slice or a row goes to the node in the
//! graph and selects it. In a smaller window the charts shrink to their
//! cells and the list's columns narrow, then scroll.

use egui::{Color32, FontId, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui, Vec2};

use crate::editor::Editor;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::data_view::stats::humanize_milliseconds;
use crate::ui::editors::palette_color;
use crate::ui::tool_window;
use crate::ui::widgets::Tooltips;

/// The window's size when it opens, and the least it can be made.
const DEFAULT_SIZE: Vec2 = Vec2::new(1200.0, 720.0);
const MIN_SIZE: Vec2 = Vec2::new(480.0, 300.0);
/// `DonutChart.OuterRadius` and `InnerRadius`: the ring while its cell has
/// room for it. In a smaller cell it shrinks, the hole in proportion.
const OUTER_RADIUS: f32 = 150.0;
const INNER_RADIUS: f32 = 100.0;
/// The room between a chart's title and its ring.
const TITLE_GAP: f32 = 4.0;
/// The least the charts are left across while the list takes more than
/// its half.
const CHARTS_LEAST: f32 = 160.0;
/// The slices' outline (`Color.FromRgb(45, 15, 65)`).
const SLICE_OUTLINE: Color32 = Color32::from_rgb(45, 15, 65);
/// A slice under the pointer (`IsMouseOver` → `Opacity` 0.8).
const HOVER_OPACITY: f32 = 0.8;
/// `statview`'s columns, their headers, the least the name column narrows
/// to (its names cut short) and the room kept between columns as they
/// narrow.
const NAME_COLUMN: f32 = 250.0;
const SIZE_COLUMN: f32 = 140.0;
const TIME_COLUMN: f32 = 180.0;
const HEADERS: [&str; 3] = ["Name", "Size in Memory", "Build Time"];
const NAME_LEAST: f32 = 100.0;
const COLUMN_GAP: f32 = 8.0;
/// `ListViewItem`'s padding around 14-point text.
const ROW_PADDING: f32 = 4.0;
/// The list's border and the room either side of its columns.
const LIST_CHROME: f32 = 2.0 + 2.0 + ROW_PADDING * 2.0;
/// The strip along the bottom the hint takes, at least, and the margin
/// round it.
const HINT_LEAST: f32 = 36.0;
const HINT_MARGIN: f32 = 4.0;
/// `ListBox-Background`, and a chosen row (`DarkGray2`).
const LIST_BACKGROUND: Color32 = Color32::from_rgb(0x16, 0x16, 0x16);
const SELECTED_ROW: Color32 = Color32::from_rgb(0x32, 0x32, 0x32);
/// The charts' colours, dark violet through red and orange to pale yellow
/// (matplotlib's magma, CC0, from 0.2 to 1), sampled once a slice.
const RAMP: [[u8; 3]; 9] = [
    [59, 15, 112],
    [100, 26, 128],
    [140, 41, 129],
    [183, 55, 121],
    [222, 73, 104],
    [247, 112, 92],
    [254, 159, 109],
    [254, 207, 146],
    [252, 253, 191],
];
const HINT: &str = "Click any Chart item to select a node in the List. Double-click any Chart or List item to select the node in the graph.";

/// A slice: the node's name, its share and id.
#[derive(Clone, Debug, PartialEq)]
struct Slice {
    name: String,
    value: f64,
    id: i32,
}

/// A row of the list: name, megabytes, milliseconds and the node (0 for
/// TOTAL).
#[derive(Clone, Debug, PartialEq)]
struct Row {
    name: String,
    megabytes: i64,
    milliseconds: f64,
    id: i32,
}

/// `ChartMode`.
#[derive(Clone, Copy, Debug, PartialEq)]
enum ChartMode {
    Time,
    Memory,
}

/// What a chart or the list was asked.
enum Picked {
    Nothing,
    Selected(i32),
    GoTo(i32),
}

/// Where the window's parts go.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Parts {
    times: Rect,
    sizes: Rect,
    list: Rect,
    hint: Rect,
}

/// The window's figures, gathered when it opens (`StatisticsWindow_Loaded`).
#[derive(Clone, Debug, PartialEq)]
pub struct StatisticsWindow {
    times: Vec<Slice>,
    sizes: Vec<Slice>,
    rows: Vec<Row>,
    selected: Option<i32>,
    open: bool,
}

impl StatisticsWindow {
    pub fn new(editor: &Editor) -> Self {
        let mut times = Vec::new();
        let mut sizes = Vec::new();
        let mut rows = Vec::new();
        for node in editor.document.terrain().nodes.values() {
            let bytes = editor.node_caches.get(&node.id).map_or(0, |cache| cache.bytes);
            let megabytes = (bytes / 1024 / 1024) as i64;
            let milliseconds = editor.status(node.id).milliseconds as f64;
            rows.push(Row { name: node.name.clone(), megabytes, milliseconds, id: node.id });
            times.push(Slice { name: node.name.clone(), value: milliseconds, id: node.id });
            sizes.push(Slice { name: node.name.clone(), value: megabytes as f64, id: node.id });
        }
        let total_megabytes = rows.iter().map(|row| row.megabytes).sum();
        let total_milliseconds = rows.iter().map(|row| row.milliseconds).sum();
        rows.push(Row { name: "TOTAL".to_string(), megabytes: total_megabytes, milliseconds: total_milliseconds, id: 0 });
        // `OrderByDescending`, which keeps ties in order.
        times.sort_by(|left, right| right.value.total_cmp(&left.value));
        sizes.sort_by(|left, right| right.value.total_cmp(&left.value));
        rows.sort_by(|left, right| right.milliseconds.total_cmp(&left.milliseconds));
        Self { times, sizes, rows, selected: None, open: true }
    }
}

/// The colour of slice `index` of `count`: the ramp at the slice's middle,
/// as a ramp shrunk to `count` pixels gives it.
fn slice_colour(index: usize, count: usize) -> Color32 {
    let position = (index as f32 + 0.5) / count.max(1) as f32;
    let scaled = position.clamp(0.0, 1.0) * (RAMP.len() - 1) as f32;
    let lower = (scaled.floor() as usize).min(RAMP.len() - 2);
    let fraction = scaled - lower as f32;
    let mix = |channel: usize| {
        let from = RAMP[lower][channel] as f32;
        let to = RAMP[lower + 1][channel] as f32;
        (from + (to - from) * fraction).round() as u8
    };
    Color32::from_rgb(mix(0), mix(1), mix(2))
}

/// Humanizer's `ByteSize.Humanize()` of so many megabytes: the largest
/// unit holding at least one, to two decimals (`0.##`).
pub(crate) fn humanize_megabytes(megabytes: i64) -> String {
    let bytes = megabytes as f64 * 1024.0 * 1024.0;
    let units = [("TB", 1024f64.powi(4)), ("GB", 1024f64.powi(3)), ("MB", 1024f64.powi(2)), ("KB", 1024.0), ("B", 1.0)];
    for (symbol, size) in units {
        if bytes.abs() >= size {
            let rounded = (bytes / size * 100.0).round() / 100.0;
            let digits = format!("{rounded:.2}");
            let trimmed = digits.trim_end_matches('0').trim_end_matches('.');
            return format!("{trimmed} {symbol}");
        }
    }
    "0 b".to_string()
}

/// A slice's caption under its name: its time (`TimeSpan.Humanize` with
/// "milliseconds" shortened to "ms") or its size.
fn slice_text(slice: &Slice, mode: ChartMode) -> String {
    match mode {
        ChartMode::Time => humanize_milliseconds(slice.value as f32).replace("milliseconds", "ms"),
        ChartMode::Memory => humanize_megabytes(slice.value as i64),
    }
}

/// `{0:hh\:mm\:ss\.fff}` in its four colours (`ParamsZ-Color1`, `2`, `4`
/// and `8`), the separators in the list's own.
fn time_cell(milliseconds: f64) -> Vec<(String, Color32)> {
    let whole = milliseconds.max(0.0).floor() as u64;
    let hours = whole / 3_600_000 % 24;
    let minutes = whole / 60_000 % 60;
    let seconds = whole / 1000 % 60;
    let fraction = whole % 1000;
    vec![
        (format!("{hours:02}"), palette_color(2, 0)),
        (":".to_string(), theme::LABEL_DEFAULT),
        (format!("{minutes:02}"), palette_color(2, 1)),
        (":".to_string(), theme::LABEL_DEFAULT),
        (format!("{seconds:02}"), palette_color(2, 3)),
        (".".to_string(), theme::LABEL_DEFAULT),
        (format!("{fraction:03}"), palette_color(2, 7)),
    ]
}

/// The window while it is open; what is double-clicked is gone to.
pub fn show(ctx: &egui::Context, window: &mut Option<StatisticsWindow>, commands: &mut Vec<Command>) {
    let Some(statistics) = window.as_mut() else {
        return;
    };
    let mut open = statistics.open;
    let mut picked = Vec::new();
    let mut frame = egui::Frame::window(&ctx.global_style());
    frame.fill = theme::WINDOW_BACKGROUND;
    frame.inner_margin = egui::Margin::same(8);
    let mut statistics_window = egui::Window::new("Statistics");
    statistics_window = statistics_window.open(&mut open);
    statistics_window = statistics_window.collapsible(false);
    statistics_window = statistics_window.frame(frame);
    // Before `place`, which can fix the size for a frame.
    statistics_window = statistics_window.min_size(MIN_SIZE);
    statistics_window = tool_window::place(ctx, statistics_window, Id::new("statistics-window"), DEFAULT_SIZE);
    statistics_window.show(ctx, |ui| {
        let area = ui.available_rect_before_wrap();
        ui.allocate_rect(area, Sense::hover());
        // The hint along the bottom (margin 4), wrapped to the window.
        let mut job = egui::text::LayoutJob::simple(HINT.to_string(), theme::interface(12.0), theme::LABEL_DEFAULT, (area.width() - HINT_MARGIN * 2.0).max(1.0));
        job.halign = egui::Align::Center;
        let hint = ui.fonts_mut(|fonts| fonts.layout_job(job));
        let least = least_columns(ui, &statistics.rows);
        let parts = parts(area, hint.size().y, least.iter().sum::<f32>() + LIST_CHROME);
        picked.push(chart_cell(ui, parts.times, "Build Time", &statistics.times, ChartMode::Time));
        picked.push(chart_cell(ui, parts.sizes, "Cache Size", &statistics.sizes, ChartMode::Memory));
        picked.push(statistics_list(ui, parts.list, &statistics.rows, least, &mut statistics.selected));
        let hint_position = Pos2::new(parts.hint.center().x, parts.hint.bottom() - HINT_MARGIN - hint.size().y);
        ui.painter_at(parts.hint).galley(hint_position, hint, theme::LABEL_DEFAULT);
    });
    for pick in picked {
        match pick {
            Picked::Nothing => {}
            Picked::Selected(id) => statistics.selected = Some(id),
            Picked::GoTo(id) => {
                // `Chart_ItemDoubleClicked` chooses the row too. TOTAL has
                // no node.
                statistics.selected = Some(id);
                if id != 0 {
                    commands.push(Command::GoToNode(id));
                }
            }
        }
    }
    statistics.open = open;
    if !open {
        *window = None;
    }
}

/// Where the parts go in `area`: the two charts down the left, one over the
/// other, the list on the right, and under both the hint, `hint_height`
/// high. The list takes half the width, or more while its columns need
/// `list_least` and the charts keep `CHARTS_LEAST`.
fn parts(area: Rect, hint_height: f32, list_least: f32) -> Parts {
    let footer = (hint_height + HINT_MARGIN * 2.0).max(HINT_LEAST).min(area.height());
    let hint = area.with_min_y(area.bottom() - footer);
    let list_width = (area.width() / 2.0).max(list_least.min(area.width() - CHARTS_LEAST));
    let split = area.right() - list_width;
    let charts = Rect::from_min_max(area.min, Pos2::new(split, hint.top()));
    let middle = charts.center().y;
    Parts {
        times: charts.with_max_y(middle),
        sizes: charts.with_min_y(middle),
        list: Rect::from_min_max(Pos2::new(split, area.top()), Pos2::new(area.right(), hint.top())),
        hint,
    }
}

/// A chart's ring in `cell`, under a title `title_height` high: its centre
/// and outer radius, `OUTER_RADIUS` while the cell has room and as large
/// as fits in it when not.
fn ring(cell: Rect, title_height: f32) -> (Pos2, f32) {
    let room = cell.with_min_y((cell.top() + title_height + TITLE_GAP).min(cell.bottom()));
    // Half the outline's width inside it too.
    let radius = ((room.width().min(room.height()) - 1.0) / 2.0).clamp(0.0, OUTER_RADIUS);
    (room.center(), radius)
}

/// The radius of the hole in a ring of `radius`.
fn hole_radius(radius: f32) -> f32 {
    radius * INNER_RADIUS / OUTER_RADIUS
}

/// A chart's cell: its title (14, bold) at the top, the donut under it
/// and, over its hole, the slice under the pointer, all kept to the cell.
fn chart_cell(ui: &mut Ui, cell: Rect, title: &str, slices: &[Slice], mode: ChartMode) -> Picked {
    let painter = ui.painter_at(cell);
    let title_galley = painter.layout_no_wrap(title.to_string(), theme::semibold(14.0), theme::LABEL_EMPHASIS);
    let (centre, radius) = ring(cell, title_galley.size().y);
    let title_position = Pos2::new(cell.center().x - title_galley.size().x / 2.0, cell.top());
    painter.galley(title_position, title_galley, theme::LABEL_EMPHASIS);
    let total: f64 = slices.iter().map(|slice| slice.value).sum();
    if total <= 0.0 || !total.is_finite() || radius <= 0.0 {
        return Picked::Nothing;
    }
    let chart_rect = Rect::from_center_size(centre, Vec2::splat(radius * 2.0));
    let response = ui.interact(chart_rect, ui.id().with(("donut", title)), Sense::click());
    let hovered = response.hover_pos().and_then(|pointer| slice_at(pointer, centre, radius, slices, total));

    let mut start = 0.0f64;
    for (index, slice) in slices.iter().enumerate() {
        let sweep = slice.value / total * 360.0;
        let colour = slice_colour(index, slices.len());
        let fill = if hovered == Some(index) { colour.gamma_multiply(HOVER_OPACITY) } else { colour };
        paint_slice(&painter, centre, radius, start as f32, sweep as f32, fill);
        start += sweep;
    }
    if let Some(index) = hovered {
        let slice = &slices[index];
        let caption = format!("{}\n{}", slice.name, slice_text(slice, mode));
        let hole = hole_radius(radius);
        let mut job = egui::text::LayoutJob::simple(caption.clone(), theme::interface(12.0), theme::LABEL_DEFAULT, (hole * 2.0 - 16.0).max(1.0));
        job.halign = egui::Align::Center;
        let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
        // In the hole while it holds it, else beside the pointer.
        if (galley.size() / 2.0).length() <= hole {
            let position = Pos2::new(centre.x, centre.y - galley.size().y / 2.0);
            painter.galley(position, galley, theme::LABEL_DEFAULT);
        } else {
            response.clone().tooltip_at_pointer(caption);
        }
    }
    let Some(index) = hovered else {
        return Picked::Nothing;
    };
    let id = slices[index].id;
    if response.double_clicked() {
        return Picked::GoTo(id);
    }
    if response.clicked() {
        return Picked::Selected(id);
    }
    Picked::Nothing
}

/// The slice under `pointer`: in the ring of `radius`, by its angle
/// clockwise from the top.
fn slice_at(pointer: Pos2, centre: Pos2, radius: f32, slices: &[Slice], total: f64) -> Option<usize> {
    let offset = pointer - centre;
    let distance = offset.length();
    if !(hole_radius(radius)..=radius).contains(&distance) {
        return None;
    }
    let angle = (offset.y.atan2(offset.x).to_degrees() as f64 + 90.0).rem_euclid(360.0);
    let mut start = 0.0;
    for (index, slice) in slices.iter().enumerate() {
        let sweep = slice.value / total * 360.0;
        if angle >= start && angle < start + sweep {
            return Some(index);
        }
        start += sweep;
    }
    None
}

/// `GetPoint`: on the circle of `radius`, `angle` degrees clockwise from
/// the top.
fn point_at(centre: Pos2, angle: f32, radius: f32) -> Pos2 {
    let radians = (angle - 90.0).to_radians();
    Pos2::new(centre.x + radius * radians.cos(), centre.y + radius * radians.sin())
}

/// A slice of the ring of `radius` from `start` through `sweep` degrees,
/// filled and outlined.
fn paint_slice(painter: &Painter, centre: Pos2, radius: f32, start: f32, sweep: f32, fill: Color32) {
    if sweep <= 0.0 {
        return;
    }
    let steps = ((sweep / 2.0).ceil() as usize).max(2);
    let mut outer = Vec::with_capacity(steps + 1);
    let mut inner = Vec::with_capacity(steps + 1);
    for step in 0..=steps {
        let angle = start + sweep * step as f32 / steps as f32;
        outer.push(point_at(centre, angle, radius));
        inner.push(point_at(centre, angle, hole_radius(radius)));
    }
    let mut mesh = egui::Mesh::default();
    for (outer_point, inner_point) in outer.iter().zip(&inner) {
        mesh.colored_vertex(*outer_point, fill);
        mesh.colored_vertex(*inner_point, fill);
    }
    for step in 0..steps as u32 {
        let first = step * 2;
        mesh.add_triangle(first, first + 1, first + 2);
        mesh.add_triangle(first + 1, first + 3, first + 2);
    }
    painter.add(Shape::mesh(mesh));
    let mut outline = outer;
    outline.extend(inner.into_iter().rev());
    painter.add(Shape::closed_line(outline, Stroke::new(1.0, SLICE_OUTLINE)));
}

/// A row's size as the list shows it.
fn size_text(row: &Row) -> String {
    format!("{} MB", row.megabytes)
}

/// The least each of `statview`'s columns narrows to: the name to
/// `NAME_LEAST`, the size and the time to their header or their widest
/// cell, whichever is wider.
fn least_columns(ui: &Ui, rows: &[Row]) -> [f32; 3] {
    let width = |text: String, font: FontId| ui.painter().layout_no_wrap(text, font, theme::LABEL_DEFAULT).size().x;
    let header = |index: usize| width(HEADERS[index].to_string(), theme::interface(14.0)) + 2.0;
    let widest_size = rows.iter().map(|row| width(size_text(row), theme::monospace(14.0))).fold(0.0, f32::max);
    // `hh:mm:ss.fff` is as wide whatever the time.
    let time: String = time_cell(0.0).into_iter().map(|(part, _)| part).collect();
    let time_width = width(time, theme::monospace(14.0));
    [NAME_LEAST, header(1).max(widest_size + 4.0) + COLUMN_GAP, header(2).max(time_width + 4.0) + COLUMN_GAP]
}

/// `statview`'s columns in `available` points: as wide as written while
/// there is room, then narrowed together, each by its share of what they
/// can spare, down to `least`.
fn column_widths(available: f32, least: [f32; 3]) -> [f32; 3] {
    let written = [NAME_COLUMN, SIZE_COLUMN, TIME_COLUMN];
    let full: [f32; 3] = std::array::from_fn(|index| written[index].max(least[index]));
    let spare: f32 = (0..3).map(|index| full[index] - least[index]).sum();
    let over = (full.iter().sum::<f32>() - available).clamp(0.0, spare);
    let narrowed = if spare > 0.0 { over / spare } else { 0.0 };
    std::array::from_fn(|index| full[index] - (full[index] - least[index]) * narrowed)
}

/// `statview`: the column headers, then a row a node (14 points, padding
/// 4); a click chooses a row, a double click goes to its node. Its columns
/// narrow with it down to `least`, and past that it scrolls across, the
/// headers with it.
fn statistics_list(ui: &mut Ui, rect: Rect, rows: &[Row], least: [f32; 3], selected: &mut Option<i32>) -> Picked {
    ui.painter().rect_filled(rect, 0.0, LIST_BACKGROUND);
    ui.painter().rect_stroke(rect, 0.0, Stroke::new(1.0, theme::LISTBOX_BORDER), egui::StrokeKind::Inside);
    let font = theme::interface(14.0);
    let mono = theme::monospace(14.0);
    let line_height = ui.painter().layout_no_wrap("Ag".to_string(), font.clone(), theme::LABEL_DEFAULT).size().y;
    let row_height = line_height + ROW_PADDING * 2.0;
    let header_rect = Rect::from_min_size(rect.min + Vec2::new(1.0, 1.0), Vec2::new(rect.width() - 2.0, row_height));
    let [name_column, size_column, time_column] = column_widths(rect.width() - LIST_CHROME, least);
    let content_width = name_column + size_column + time_column + LIST_CHROME - 2.0;

    let body = Rect::from_min_max(Pos2::new(rect.left() + 1.0, header_rect.bottom()), rect.max - Vec2::splat(1.0));
    let mut body_ui = ui.new_child(egui::UiBuilder::new().max_rect(body));
    let mut picked = Picked::Nothing;
    let mut scroll = egui::ScrollArea::both();
    scroll = scroll.id_salt("statistics-list");
    scroll = scroll.auto_shrink([false, false]);
    let scrolled = scroll.show(&mut body_ui, |ui| {
        ui.spacing_mut().item_spacing = Vec2::ZERO;
        for row in rows {
            let row_width = ui.available_width().max(content_width);
            let (row_rect, response) = ui.allocate_exact_size(Vec2::new(row_width, row_height), Sense::click());
            if *selected == Some(row.id) {
                ui.painter().rect_filled(row_rect, 0.0, SELECTED_ROW);
            }
            let text_y = row_rect.center().y;
            let name_left = row_rect.left() + 2.0 + ROW_PADDING + 2.0;
            paint_cell(ui, &row.name, font.clone(), theme::LABEL_DEFAULT, Pos2::new(name_left, text_y), false, name_column - 8.0);
            let size_right = row_rect.left() + 2.0 + ROW_PADDING + name_column + size_column - 4.0;
            paint_cell(ui, &size_text(row), mono.clone(), theme::LABEL_DEFAULT, Pos2::new(size_right, text_y), true, size_column);
            let time_right = size_right + time_column;
            paint_time(ui, row.milliseconds, mono.clone(), Pos2::new(time_right, text_y));
            if response.double_clicked() {
                picked = Picked::GoTo(row.id);
            } else if response.clicked() {
                picked = Picked::Selected(row.id);
            }
        }
    });

    let header_painter = ui.painter_at(header_rect);
    let mut x = header_rect.left() + 2.0 + ROW_PADDING - scrolled.state.offset.x;
    for (header, width) in HEADERS.into_iter().zip([name_column, size_column, time_column]) {
        let galley = header_painter.layout_no_wrap(header.to_string(), font.clone(), theme::LABEL_EMPHASIS);
        header_painter.galley(Pos2::new(x + 2.0, header_rect.center().y - galley.size().y / 2.0), galley, theme::LABEL_EMPHASIS);
        x += width;
    }
    picked
}

/// One cell's text, left-aligned from `anchor` or right-aligned to it.
fn paint_cell(ui: &Ui, text: &str, font: FontId, colour: Color32, anchor: Pos2, right: bool, width: f32) {
    let mut job = egui::text::LayoutJob::simple_singleline(text.to_string(), font, colour);
    job.wrap.max_width = width;
    job.wrap.max_rows = 1;
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let left = if right { anchor.x - galley.size().x } else { anchor.x };
    ui.painter().galley(Pos2::new(left, anchor.y - galley.size().y / 2.0), galley, colour);
}

/// The build time, right-aligned to `anchor`, in its colours.
fn paint_time(ui: &Ui, milliseconds: f64, font: FontId, anchor: Pos2) {
    let mut job = egui::text::LayoutJob::default();
    for (part, colour) in time_cell(milliseconds) {
        let format = egui::TextFormat::simple(font.clone(), colour);
        job.append(&part, 0.0, format);
    }
    let galley = ui.fonts_mut(|fonts| fonts.layout_job(job));
    let position = Pos2::new(anchor.x - galley.size().x, anchor.y - galley.size().y / 2.0);
    ui.painter().galley(position, galley, theme::LABEL_DEFAULT);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::epaint::ClippedShape;
    use egui::{AreaState, RawInput};

    /// The screen, in points, of 1600 × 960 at the default 120%.
    const LARGE: Vec2 = Vec2::new(1333.0, 800.0);
    /// 800 × 600 and 640 × 480 at 120%.
    const SMALL_SCREENS: [Vec2; 2] = [Vec2::new(666.0, 500.0), Vec2::new(533.0, 400.0)];

    #[test]
    fn sizes_and_times_read_as_written() {
        assert_eq!(humanize_megabytes(0), "0 b");
        assert_eq!(humanize_megabytes(16), "16 MB");
        assert_eq!(humanize_megabytes(1536), "1.5 GB");
        let parts: Vec<String> = time_cell(3_723_004.9).into_iter().map(|(text, _)| text).collect();
        assert_eq!(parts.concat(), "01:02:03.004");
    }

    #[test]
    fn slices_are_found_clockwise_from_the_top() {
        let slices = vec![Slice { name: "a".into(), value: 3.0, id: 1 }, Slice { name: "b".into(), value: 1.0, id: 2 }];
        let centre = Pos2::new(200.0, 200.0);
        // Right of the centre is 90 degrees round: the first slice (0-270).
        assert_eq!(slice_at(Pos2::new(325.0, 200.0), centre, OUTER_RADIUS, &slices, 4.0), Some(0));
        // Left is 270: the second.
        assert_eq!(slice_at(Pos2::new(75.0, 199.0), centre, OUTER_RADIUS, &slices, 4.0), Some(1));
        assert_eq!(slice_at(Pos2::new(200.0, 200.0), centre, OUTER_RADIUS, &slices, 4.0), None);
        // A smaller ring, its hole in proportion.
        assert_eq!(slice_at(Pos2::new(250.0, 200.0), centre, 60.0, &slices, 4.0), Some(0));
        assert_eq!(slice_at(Pos2::new(235.0, 200.0), centre, 60.0, &slices, 4.0), None);
        assert_eq!(slice_at(Pos2::new(325.0, 200.0), centre, 60.0, &slices, 4.0), None);
        assert_eq!(slice_colour(0, 1), Color32::from_rgb(222, 73, 104));
    }

    #[test]
    fn rings_fit_their_cells_under_their_titles() {
        let title = 17.0;
        // With room, as written.
        let roomy = Rect::from_min_size(Pos2::new(8.0, 40.0), Vec2::new(592.0, 340.0));
        assert_eq!(ring(roomy, title).1, OUTER_RADIUS);
        for size in [Vec2::new(592.0, 340.0), Vec2::new(320.0, 180.0), Vec2::new(180.0, 120.0), Vec2::new(300.0, 60.0), Vec2::new(10.0, 10.0)] {
            let cell = Rect::from_min_size(Pos2::new(8.0, 40.0), size);
            let (centre, radius) = ring(cell, title);
            assert!(radius <= OUTER_RADIUS);
            // Too small for a ring: none is drawn.
            if radius <= 0.0 {
                continue;
            }
            let square = Rect::from_center_size(centre, Vec2::splat(radius * 2.0));
            assert!(cell.contains_rect(square), "{square:?} is outside {cell:?}");
            assert!(square.top() >= cell.top() + title, "{square:?} covers the title in {cell:?}");
        }
    }

    #[test]
    fn columns_narrow_to_what_they_hold() {
        let least = [NAME_LEAST, 110.0, 115.0];
        let written = [NAME_COLUMN, SIZE_COLUMN, TIME_COLUMN];
        assert_eq!(column_widths(1000.0, least), written);
        assert_eq!(column_widths(written.iter().sum(), least), written);
        assert_eq!(column_widths(100.0, least), least);
        let narrowed = column_widths(400.0, least);
        assert!((narrowed.iter().sum::<f32>() - 400.0).abs() < 0.01, "{narrowed:?}");
        for index in 0..3 {
            assert!(least[index] < narrowed[index] && narrowed[index] < written[index], "{narrowed:?}");
        }
        // A column that needs more than written keeps what it needs.
        assert_eq!(column_widths(1000.0, [NAME_LEAST, 160.0, 115.0])[1], 160.0);
    }

    #[test]
    fn parts_keep_apart() {
        let large = Rect::from_min_size(Pos2::new(8.0, 40.0), Vec2::new(1184.0, 680.0));
        let roomy = parts(large, 15.0, 340.0);
        // With room, as written: halves, the list over a 36-point strip.
        assert_eq!(roomy.list, Rect::from_min_max(Pos2::new(large.center().x, large.top()), large.max - Vec2::new(0.0, 36.0)));
        assert_eq!(roomy.times.right(), large.center().x);
        for area in [large, Rect::from_min_size(Pos2::ZERO, Vec2::new(517.0, 270.0)), Rect::from_min_size(Pos2::ZERO, Vec2::new(464.0, 240.0))] {
            let parts = parts(area, 30.0, 340.0);
            let all = [parts.times, parts.sizes, parts.list, parts.hint];
            for (index, part) in all.iter().enumerate() {
                assert!(area.contains_rect(*part), "{part:?} is outside {area:?}");
                for other in &all[index + 1..] {
                    assert!(part.intersect(*other).area() <= 0.0, "{part:?} overlaps {other:?}");
                }
            }
            assert!(parts.hint.height() >= 30.0 + HINT_MARGIN * 2.0);
            assert!(parts.list.width() >= (340.0f32).min(area.width() - CHARTS_LEAST), "{:?} is narrower than its columns", parts.list);
        }
    }

    fn context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        context
    }

    /// A project's figures, a node at a time, as `new` gathers them.
    fn statistics() -> StatisticsWindow {
        let names = ["Mountain", "Erosion", "Thermal Weathering", "Rivers", "Snow", "Combine", "Height Selector", "Sediments", "Fault"];
        let mut times = Vec::new();
        let mut sizes = Vec::new();
        let mut rows = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let id = index as i32 + 1;
            let milliseconds = 61_234.0 / (index + 1) as f64;
            let megabytes = 2048 / (index as i64 + 1);
            rows.push(Row { name: name.to_string(), megabytes, milliseconds, id });
            times.push(Slice { name: name.to_string(), value: milliseconds, id });
            sizes.push(Slice { name: name.to_string(), value: megabytes as f64, id });
        }
        let total_megabytes = rows.iter().map(|row| row.megabytes).sum();
        let total_milliseconds = rows.iter().map(|row| row.milliseconds).sum();
        rows.insert(0, Row { name: "TOTAL".to_string(), megabytes: total_megabytes, milliseconds: total_milliseconds, id: 0 });
        StatisticsWindow { times, sizes, rows, selected: None, open: true }
    }

    /// A frame at this screen size. Returns what was painted and the
    /// window's rect.
    fn frame(context: &egui::Context, screen: Vec2, window: &mut Option<StatisticsWindow>) -> (Vec<ClippedShape>, Rect) {
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, screen)), ..Default::default() };
        let mut output = context.run_ui(input, |ui| {
            show(ui.ctx(), window, &mut Vec::new());
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let area = AreaState::load(context, Id::new("statistics-window")).filter(|area| area.size.is_some());
        (shapes, area.map(|area| area.rect()).unwrap_or(Rect::NOTHING))
    }

    /// Frames enough for egui's first-frame sizing to settle and the
    /// window to fade in.
    fn settle(context: &egui::Context, screen: Vec2, window: &mut Option<StatisticsWindow>) -> (Vec<ClippedShape>, Rect) {
        for _ in 0..30 {
            frame(context, screen, window);
        }
        frame(context, screen, window)
    }

    #[test]
    fn small_windows_keep_their_charts_titles_hint_and_columns_apart() {
        let context = context();
        let mut window = Some(statistics());
        let (_, before) = settle(&context, LARGE, &mut window);
        for screen in SMALL_SCREENS {
            let (shapes, rect) = settle(&context, screen, &mut window);
            assert!(rect.width() >= MIN_SIZE.x.min(tool_window::body(&context).width()) - 0.5, "{rect:?} at {screen:?} is narrower than its least");
            // The rings: their slices' meshes, in the charts' colours, each
            // chart's kept to its cell.
            let count = window.as_ref().map_or(0, |window| window.times.len());
            let mut rings: Vec<(Rect, Rect)> = Vec::new();
            for shape in &shapes {
                if let Shape::Mesh(mesh) = &shape.shape
                    && mesh.vertices.first().is_some_and(|vertex| (0..count).any(|index| slice_colour(index, count) == vertex.color))
                {
                    let bounds = mesh.calc_bounds();
                    match rings.iter_mut().find(|(cell, _)| *cell == shape.clip_rect) {
                        Some((_, ring)) => *ring = ring.union(bounds),
                        None => rings.push((shape.clip_rect, bounds)),
                    }
                }
            }
            assert_eq!(rings.len(), 2, "at {screen:?}: {rings:?}");
            let [(_, times), (_, sizes)] = [rings[0], rings[1]];
            assert!(times.intersect(sizes).area() <= 0.0, "at {screen:?} the rings overlap: {times:?}, {sizes:?}");
            for (cell, ring) in &rings {
                assert!(cell.contains_rect(*ring), "at {screen:?} {ring:?} runs out of its cell {cell:?}");
                assert!(rect.contains_rect(*ring), "at {screen:?} {ring:?} runs out of the window {rect:?}");
            }
            let texts: Vec<(&str, Rect, Rect)> = shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    Shape::Text(text) => Some((text.galley.text(), text.visual_bounding_rect(), shape.clip_rect)),
                    _ => None,
                })
                .collect();
            for (text, bounds, clip) in &texts {
                if clip.contains_rect(*bounds) {
                    for ring in [times, sizes] {
                        // Only the caption of a hovered slice sits in a ring.
                        assert!(ring.intersect(*bounds).area() <= 0.0, "at {screen:?} a ring covers {text:?}");
                    }
                }
            }
            // The titles, the hint and the list's headers and figures are
            // whole.
            let whole = |wanted: &dyn Fn(&str) -> bool| texts.iter().filter(|(text, ..)| wanted(text)).all(|(_, bounds, clip)| clip.expand(0.5).contains_rect(*bounds));
            let count = |wanted: &dyn Fn(&str) -> bool| texts.iter().filter(|(text, ..)| wanted(text)).count();
            for title in ["Statistics", "Build Time", "Cache Size", "Name", "Size in Memory"] {
                assert!(count(&|text| text == title) >= 1, "at {screen:?} {title:?} is not shown");
                assert!(whole(&|text| text == title), "at {screen:?} {title:?} is cut off");
            }
            assert_eq!(count(&|text| text == HINT), 1, "at {screen:?} the hint is not shown");
            assert!(whole(&|text| text == HINT), "at {screen:?} the hint is cut off");
            let sizes_shown = |text: &str| text.ends_with(" MB");
            let times_shown = |text: &str| text.len() == 12 && text.as_bytes()[2] == b':' && text.as_bytes()[8] == b'.';
            assert!(count(&sizes_shown) >= 5 && count(&times_shown) >= 5, "at {screen:?} the list's figures are not shown");
            // Across, that is: the last row can be part scrolled out of
            // the bottom.
            for (text, bounds, clip) in texts.iter().filter(|(text, ..)| sizes_shown(text) || times_shown(text)) {
                if clip.y_range().intersects(bounds.y_range()) {
                    let across = clip.x_range().expand(0.5);
                    assert!(across.contains(bounds.left()) && across.contains(bounds.right()), "at {screen:?} {text:?} is cut off: {bounds:?} in {clip:?}");
                }
            }
        }
        let (_, after) = settle(&context, LARGE, &mut window);
        assert!((after.min - before.min).length() <= 1.0 && (after.size() - before.size()).length() <= 1.0, "{after:?} did not go back to {before:?}");
    }
}
