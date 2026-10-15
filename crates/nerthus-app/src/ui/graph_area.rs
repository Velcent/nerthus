//! The graph grid (`GraphGrid`): the graph area (`GraphArea`: graph tabs
//! along the top, the node graph, the toolbox over its corner, the build
//! clock and progress at the bottom), the Data View beside it while open,
//! and the graph toolbar on the right.

use egui::{CornerRadius, CursorIcon, Pos2, Rect, Sense, Shape, Stroke, Ui, UiBuilder, Vec2};

use crate::document::{CombineKind, GraphTabInfo};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::graph_canvas::elided_galley;
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_popups::swatch_grid;
use crate::ui::widgets::{checked_icon_item, dialog, dialog_button, icon_button, icon_menu, menu_caption, menu_item, menu_item_enabled, menu_rows, menu_separator, menu_style, submenu, toolbar_background_corners, toolbar_separator, vertical_toolbar, Tooltips};
use crate::ui::{data_view, graph_canvas, menus, toolbox, viewport_area, UiState};
use crate::user::ToolboxStyle;

/// Height of the tab strip (`GraphTabsGrid`) and of a tab (`GraphTabItem`).
const TABS_HEIGHT: f32 = 26.0;
const TAB_HEIGHT: f32 = 25.0;
/// `TabBorderThickness`: tab outlines, the line under the tabs and the
/// graph's border.
const TAB_BORDER: f32 = 1.5;
/// `GraphBorder`'s top margin below the tabs.
const GRAPH_TOP: f32 = 25.0;
/// A graph tab's width while the strip has room for every tab: its name
/// with 24 either side, at least 135.
const TAB_PADDING: f32 = 48.0;
const TAB_WIDTH: f32 = 135.0;
/// How narrow tabs get while the strip is short of room: the colour bar
/// and the start of the name, cut with "…". Past that the tabs scroll.
const TAB_MIN_WIDTH: f32 = 90.0;
/// A tab's name starts 24 in and stays 12 clear of its right edge.
const TAB_NAME_LEFT: f32 = 24.0;
const TAB_NAME_RIGHT: f32 = 12.0;
/// The tab list button after the tabs while they overflow the strip.
const TAB_LIST_BUTTON: f32 = 24.0;
/// `BookmarkButton`: padding 2 round a 6 by 12 bar, margin 2 either side.
const BOOKMARK_BUTTON: Vec2 = Vec2::new(6.0 + 4.0, 12.0 + 4.0);
const BOOKMARK_MARGIN: f32 = 2.0;
/// What the tabs leave clear before the bookmark bars.
const BOOKMARKS_GAP: f32 = 4.0;
/// `GraphToolbarBorder` (`Toolbar`): a `Toolbar.ButtonSize` button,
/// `Toolbar.Padding` and a border of 1.5 each side.
fn toolbar_width(sizes: &theme::Sizes) -> f32 {
    sizes.toolbar_button + 2.0 * (sizes.toolbar_padding[0] + 1.5)
}
/// Each area's margin in the grid.
const AREA_MARGIN: f32 = 2.0;
/// `gsVertical1`: 4 wide at the graph column's right edge.
const SPLITTER: f32 = 4.0;
/// How narrow the graph may get beside the Data View.
const MIN_GRAPH: f32 = 160.0;

pub fn show(ui: &mut Ui, graph_grid: Rect, editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    // The star columns left of the toolbar: the graph `5*`, the Data View
    // `1.05*` (at least 340) while open. The graph keeps `MIN_GRAPH`
    // first, so the Data View gives up width to it, and steps aside while
    // not even its tabs would fit (it comes back with the room).
    let toolbar_body = toolbar_width(theme::sizes(ui.ctx()));
    let toolbar_right_margin = AREA_MARGIN + viewport_area::TOOLBAR_MARGIN;
    let stars = (graph_grid.width() - toolbar_body - toolbar_right_margin).max(0.0);
    let least = data_view::least_width(ui) + AREA_MARGIN;
    let data_width = if ui_state.data_view.expanded { data_view_width(stars, ui_state.data_view.share, least) } else { 0.0 };
    let data_shown = data_width > 0.0;

    // The toolbar column (auto): margin 2, or 0, 2, 2, 2 against the open
    // Data View (`ToggleCollapse`). On the right it also keeps the viewport
    // toolbar's margin, so the two toolbars share their edges.
    let toolbar_left_margin = if data_shown { 0.0 } else { AREA_MARGIN };
    let toolbar_column_width = toolbar_left_margin + toolbar_body + toolbar_right_margin;
    let toolbar_column = Rect::from_min_max(Pos2::new(graph_grid.right() - toolbar_column_width, graph_grid.top()), graph_grid.max);
    let toolbar_rect = Rect::from_min_max(toolbar_column.min + Vec2::new(toolbar_left_margin, AREA_MARGIN), toolbar_column.max - Vec2::new(toolbar_right_margin, AREA_MARGIN));

    let split = (toolbar_column.left() - data_width).round();
    let graph_column = Rect::from_min_max(graph_grid.min, Pos2::new(split, graph_grid.bottom()));
    let area = graph_column.shrink(AREA_MARGIN);
    // What the graph draws keeps to its column: tabs and toolbox too long
    // for it are cut off at its edge, not drawn under the Data View or the
    // status bar.
    let outer_clip = ui.clip_rect();
    ui.shrink_clip_rect(area);
    ui.painter().rect_filled(area, 4.0, theme::PANEL_BACKGROUND);
    ui_state.graph.dimmed = data_view::dimmed_nodes(editor, ui_state.data_view.terrain.filter);

    // `Grid` (margin 0, 0, 1, 0): `GraphBorder` 25 down with its 1.5 sides
    // and bottom in the chosen tab's colour, the graph (margin 2) in it.
    let grid = Rect::from_min_max(area.min, area.max - Vec2::new(1.0, 0.0));
    let tabs_list = editor.document.graph_tabs();
    let selected_color = tabs_list.iter().find(|tab| tab.index == ui_state.graph.tab).map(|tab| theme::swatch_color(&tab.color)).unwrap_or(theme::TAB_BORDER);
    // `ShowHideGraphTabs`: Hide Graph Tabs with a single tab takes the tab
    // strip away. The border then reaches the top and closes round the
    // graph (1.5 all round, corners 3, `Accent-Muted-Gray`), and the
    // toolbox and the engine's outline move up with it.
    let tabs_hidden = editor.user.options.hide_tabs() && tabs_list.len() == 1;
    let graph_rect = if tabs_hidden {
        graph_border_closed(ui, grid);
        grid.shrink(TAB_BORDER + 2.0)
    } else {
        let border = Rect::from_min_max(grid.min + Vec2::new(0.0, GRAPH_TOP), grid.max);
        graph_border(ui, border, selected_color);
        Rect::from_min_max(border.min + Vec2::new(TAB_BORDER + 2.0, 2.0), border.max - Vec2::splat(TAB_BORDER + 2.0))
    };
    // `Toolbox`: margin 0, 35, 0, 0 for the menus and 0, 16, 0, 24 for the
    // panel; none without tabs. The menus end above the build strip, which
    // is drawn over the graph's bottom left after them. It is placed before
    // the graph lays out, for Extents and Selection to leave room for it,
    // and drawn over it.
    let menus = editor.user.options.toolbox_style() == ToolboxStyle::Menus;
    let mut toolbox_area = if tabs_hidden {
        grid
    } else if menus {
        Rect::from_min_max(grid.min + Vec2::new(0.0, 35.0), grid.max)
    } else {
        Rect::from_min_max(grid.min + Vec2::new(0.0, 16.0), grid.max - Vec2::new(0.0, 24.0))
    };
    if menus {
        toolbox_area.max.y = footer_top(grid).max(toolbox_area.min.y);
    }
    ui_state.graph.toolbox_rect = toolbox::place(ui, toolbox_area, &editor.user.options);
    graph_canvas::show(ui, graph_rect, editor, &mut ui_state.graph, commands);
    toolbox::show(ui, toolbox_area, editor, &mut ui_state.graph, commands);
    // `stackBookmarks` stays (margin 0, 0, 2, 0; 8 on the right without
    // tabs), in the room the tabs leave it.
    let bookmarks_margin = if tabs_hidden { 8.0 } else { 2.0 };
    let bookmarks_left = if tabs_hidden { grid.left() } else { tabs(ui, grid, bookmarks_margin, editor, ui_state, selected_color, commands) };
    bookmark_buttons(ui, grid, bookmarks_margin, bookmarks_left, editor, commands);
    footer(ui, grid, editor);
    if !editor.engine_active {
        // `EngineRect`: margin 4, 28, 4, 4; 4 all round without tabs.
        let outline_top = if tabs_hidden { 4.0 } else { 28.0 };
        engine_suspended_outline(ui, grid, outline_top);
    }
    ui.set_clip_rect(outer_clip);

    if data_shown {
        let data_column = Rect::from_min_max(Pos2::new(split, graph_column.top()), Pos2::new(toolbar_column.left(), graph_column.bottom()));
        let data_rect = Rect::from_min_max(data_column.min + Vec2::splat(AREA_MARGIN), data_column.max - Vec2::new(0.0, AREA_MARGIN));
        data_view::show(ui, data_rect, editor, &mut ui_state.data_view, &mut ui_state.graph, commands);
        data_view_splitter(ui, graph_column, stars, data_width, ui_state);
        ui_state.graph.areas.data_view = Some(data_rect);
    } else {
        ui_state.graph.areas.data_view = None;
    }
    graph_toolbar(ui, toolbar_rect, data_shown, editor, ui_state, commands);
}

/// The Data View's width in the star space: its share, at least 340 (and
/// `least`), but no more than leaves the graph `MIN_GRAPH`; 0 when that
/// is narrower than `least`, and the Data View steps aside.
fn data_view_width(stars: f32, share: f32, least: f32) -> f32 {
    let wanted = (stars * share).max(data_view::MIN_WIDTH).max(least);
    let width = wanted.min(stars - MIN_GRAPH);
    if width < least { 0.0 } else { width }
}

/// `gsVertical1` (enabled while the Data View is open): dragging it moves
/// width between the graph and the Data View, now `current` wide.
fn data_view_splitter(ui: &mut Ui, graph_column: Rect, stars: f32, current: f32, ui_state: &mut UiState) {
    let handle = Rect::from_min_max(Pos2::new(graph_column.right() - SPLITTER, graph_column.top()), graph_column.max);
    let response = ui.interact(handle, ui.id().with("data-view-splitter"), Sense::drag());
    if response.hovered() || response.dragged() {
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if !response.dragged() || stars <= 0.0 {
        return;
    }
    let wanted = current - response.drag_delta().x;
    ui_state.data_view.share = (wanted / stars).clamp(0.0, 1.0);
}

/// `GraphBorder` without tabs: 1.5 all round, corners 3, in
/// `Accent-Muted-Gray`.
fn graph_border_closed(ui: &Ui, border: Rect) {
    let stroke = Stroke::new(TAB_BORDER, theme::ACCENT_MUTED_GRAY);
    ui.painter().rect_stroke(border, 3.0, stroke, egui::StrokeKind::Inside);
}

/// `GraphBorder`: left, right and bottom sides (1.5), bottom corners 3.
fn graph_border(ui: &Ui, border: Rect, color: egui::Color32) {
    let half = TAB_BORDER / 2.0;
    let inner = Rect::from_min_max(border.min + Vec2::new(half, 0.0), border.max - Vec2::splat(half));
    let radius = 3.0 - half;
    let mut points = vec![inner.left_top()];
    let bottom_left_center = Pos2::new(inner.left() + radius, inner.bottom() - radius);
    let bottom_right_center = Pos2::new(inner.right() - radius, inner.bottom() - radius);
    // Screen angles (y down): left is pi, the bottom pi/2, right 0.
    for step in 0..=6 {
        let angle = std::f32::consts::PI - step as f32 * std::f32::consts::FRAC_PI_2 / 6.0;
        points.push(bottom_left_center + Vec2::new(angle.cos(), angle.sin()) * radius);
    }
    for step in 0..=6 {
        let angle = std::f32::consts::FRAC_PI_2 - step as f32 * std::f32::consts::FRAC_PI_2 / 6.0;
        points.push(bottom_right_center + Vec2::new(angle.cos(), angle.sin()) * radius);
    }
    points.push(inner.right_top());
    ui.painter().add(Shape::line(points, Stroke::new(TAB_BORDER, color)));
}

/// `TabShape` (`TabMiddle`) for a tab `width` wide: flared feet at the
/// bottom, corners of 4 at the top (the SVG path `GetSvgPath` builds,
/// stretched to fill the tab).
fn tab_outline(rect: Rect) -> Vec<Pos2> {
    let path_width = (rect.width() - 12.0).max(0.0) + 8.0;
    let scale_x = rect.width() / path_width;
    let scale_y = rect.height() / 25.0;
    let at = |x: f32, y: f32| Pos2::new(rect.left() + x * scale_x, rect.top() + y * scale_y);
    let arc = |center: (f32, f32), from: f32, to: f32, points: &mut Vec<Pos2>| {
        for step in 0..=6 {
            let angle = from + (to - from) * step as f32 / 6.0;
            points.push(at(center.0 + 4.0 * angle.cos(), center.1 + 4.0 * angle.sin()));
        }
    };
    let right = path_width - 8.0;
    let half_pi = std::f32::consts::FRAC_PI_2;
    let pi = std::f32::consts::PI;
    let mut points = Vec::new();
    // The left foot: from (0, 25) round to (4, 21) about (0, 21).
    arc((0.0, 21.0), half_pi, 0.0, &mut points);
    // Up to the top-left corner about (8, 4).
    arc((8.0, 4.0), pi, pi + half_pi, &mut points);
    // Across to the top-right corner about (right, 4).
    arc((right, 4.0), -half_pi, 0.0, &mut points);
    // Down to the right foot about (right + 8, 21).
    arc((right + 8.0, 21.0), pi, half_pi, &mut points);
    points
}

/// The graph tab strip between frames: how far its tabs are scrolled, and
/// what the shown tab was last brought into sight for (the tab, the
/// tabs' room and their length). It is again once one of those changes;
/// in between the wheel may scroll it away.
#[derive(Clone, Debug, Default)]
pub struct TabStrip {
    scroll: f32,
    revealed: Option<(usize, f32, f32)>,
}

/// How the tabs sit in the strip.
#[derive(Clone, Debug, PartialEq)]
struct TabLayout {
    widths: Vec<f32>,
    /// The tabs' room: the strip, less the tab list button while it shows.
    room: f32,
    /// The tabs do not fit even at `TAB_MIN_WIDTH`: they scroll, and the
    /// tab list button follows them.
    overflow: bool,
}

/// The tabs, each wanting its `natural` width, in a strip `space` wide:
/// as they are while they fit; else the widest narrowed alike until they
/// do, down to `TAB_MIN_WIDTH`; and short of that, each at that width in
/// a room that scrolls, the tab list button after it. A lone tab narrows
/// to the strip instead.
fn tab_layout(natural: &[f32], space: f32) -> TabLayout {
    let space = space.max(0.0);
    // Tabs overlap by 1 (`Margin 0,0,-1,0`).
    let budget = space + natural.len().saturating_sub(1) as f32;
    if natural.iter().sum::<f32>() <= budget {
        return TabLayout { widths: natural.to_vec(), room: space, overflow: false };
    }
    let cap = narrowed_width(natural, budget);
    if cap >= TAB_MIN_WIDTH || natural.len() < 2 {
        let widths = natural.iter().map(|width| width.min(cap.max(1.0))).collect();
        return TabLayout { widths, room: space, overflow: false };
    }
    let widths = natural.iter().map(|width| width.min(TAB_MIN_WIDTH)).collect();
    TabLayout { widths, room: (space - TAB_LIST_BUTTON).max(0.0), overflow: true }
}

/// The widest a tab may be for `natural` tabs, each at most that wide, to
/// take no more than `budget` together (whole points).
fn narrowed_width(natural: &[f32], budget: f32) -> f32 {
    let mut sorted = natural.to_vec();
    sorted.sort_by(f32::total_cmp);
    let mut narrower = 0.0;
    for (position, width) in sorted.iter().enumerate() {
        let cap = (budget - narrower) / (sorted.len() - position) as f32;
        if cap <= *width {
            return cap.floor();
        }
        narrower += width;
    }
    f32::INFINITY
}

/// Where each tab starts along the strip (they overlap by 1), and where
/// the last ends.
fn tab_spans(widths: &[f32]) -> (Vec<f32>, f32) {
    let mut starts = Vec::with_capacity(widths.len());
    let mut x = 0.0;
    for width in widths {
        starts.push(x);
        x += width - 1.0;
    }
    let end = if widths.is_empty() { 0.0 } else { x + 1.0 };
    (starts, end)
}

/// The scroll nearest `scroll` that shows `span` (along the strip) in a
/// room `room` wide; its start when it is wider than the room.
fn scroll_to_show(scroll: f32, span: (f32, f32), room: f32) -> f32 {
    let mut scroll = scroll;
    if span.1 > scroll + room {
        scroll = span.1 - room;
    }
    if span.0 < scroll {
        scroll = span.0;
    }
    scroll
}

/// How far the tabs are scrolled this frame: the shown tab brought into
/// `room` when it, the room or the tabs' length changed, then the wheel
/// over the tabs, kept to how far they reach.
fn scroll_tabs(ui: &Ui, room: Rect, tabs: &[GraphTabInfo], layout: &TabLayout, ui_state: &mut UiState) -> f32 {
    let (starts, length) = tab_spans(&layout.widths);
    let shown = ui_state.graph.tab;
    let strip = &mut ui_state.graph.tab_strip;
    if let Some(position) = tabs.iter().position(|tab| tab.index == shown) {
        let key = (shown, room.width(), length);
        if strip.revealed != Some(key) {
            let span = (starts[position], starts[position] + layout.widths[position]);
            strip.scroll = scroll_to_show(strip.scroll, span, room.width());
            strip.revealed = Some(key);
        }
    }
    let reach = (length - room.width()).max(0.0);
    if reach > 0.0 && ui.rect_contains_pointer(room) {
        let delta = ui.input(|input| input.smooth_scroll_delta);
        strip.scroll -= delta.x + delta.y;
    }
    strip.scroll = strip.scroll.clamp(0.0, reach);
    strip.scroll
}

/// The graph tabs (`GraphTabList`, 1 down): each at least 135 wide, its
/// colour bar 12 in and its name (monospace 11) 24 in. The chosen tab is
/// outlined in its colour and tinted with it; the others, and the rest of
/// the strip, stand on a line of the chosen tab's colour. Each tab's menu
/// offers Color, Rename and Delete; the strip's offers New Tab.
///
/// The tabs keep clear of the bookmark bars at the strip's end
/// (`bookmarks_margin` in) and are drawn only in the room left them.
/// Short of room they narrow, their names cut with "…" (whole under the
/// pointer); shorter still they scroll, with the wheel or the tab list
/// after them, and the shown tab is brought into sight. Returns where the
/// bookmark bars may start.
fn tabs(ui: &mut Ui, area: Rect, bookmarks_margin: f32, editor: &Editor, ui_state: &mut UiState, selected_color: egui::Color32, commands: &mut Vec<Command>) -> f32 {
    let strip = Rect::from_min_size(area.min, Vec2::new(area.width(), TABS_HEIGHT));
    let strip_response = ui.interact(strip, ui.id().with("graph-tab-strip"), Sense::click());
    strip_response.context_menu(|ui| {
        menu_style(ui);
        if menu_item(ui, Some("square-rounded-plus-2"), Some(theme::ACCENT_BLUE), "New Tab", "").clicked() {
            commands.push(Command::NewGraphTab);
        }
    });

    let tabs = editor.document.graph_tabs();
    let only_tab = tabs.len() <= 1;
    // Room for a tab and the tab list, however many bookmarks there are.
    let least = (TAB_MIN_WIDTH + TAB_LIST_BUTTON).min(strip.width());
    let space = (strip.width() - bookmarks_width(editor, bookmarks_margin)).max(least);
    let natural: Vec<f32> = tabs.iter().map(|tab| (ui.painter().layout_no_wrap(tab.name.clone(), theme::monospace(11.0), theme::LABEL_EMPHASIS).size().x + TAB_PADDING).max(TAB_WIDTH)).collect();
    let layout = tab_layout(&natural, space);
    let room = Rect::from_min_size(strip.min, Vec2::new(layout.room, TABS_HEIGHT));
    let scroll = scroll_tabs(ui, room, &tabs, &layout, ui_state);

    let line_stroke = Stroke::new(TAB_BORDER, selected_color);
    let line_y = strip.top() + 1.0 + TAB_HEIGHT - TAB_BORDER / 2.0;
    // Tabs cut off at the end of their room are cut there, and take no
    // clicks past it.
    let outer_clip = ui.clip_rect();
    let tabs_clip = Rect::from_x_y_ranges(room.x_range(), area.top()..=strip.bottom() + TAB_BORDER);
    ui.set_clip_rect(tabs_clip.intersect(outer_clip));
    let mut x = room.left() - scroll;
    for (tab, &width) in tabs.iter().zip(&layout.widths) {
        let swatch = theme::swatch_color(&tab.color);
        let selected = ui_state.graph.tab == tab.index;
        let name_width = (width - TAB_NAME_LEFT - TAB_NAME_RIGHT).max(1.0);
        let galley = elided_galley(ui.painter(), &tab.name, theme::monospace(11.0), theme::LABEL_EMPHASIS, name_width);
        let tab_rect = Rect::from_min_size(Pos2::new(x, strip.top() + 1.0), Vec2::new(width, TAB_HEIGHT));
        let response = ui.interact(tab_rect, ui.id().with(("graph-tab", tab.index)), Sense::click());
        let response = if galley.elided { response.tooltip(tab.name.clone()) } else { response };
        if response.clicked() {
            ui_state.graph.tab = tab.index;
        }
        if response.double_clicked() {
            ui_state.graph.renaming_tab = Some((tab.index, tab.name.clone()));
        }
        // `CreateContextMenuForTab`: Tab to group, Rename, Delete, the colours.
        response.context_menu(|ui| {
            menu_style(ui);
            submenu(ui, Some("stack-push"), None, "Tab to group", |ui| {
                if let Some(target) = menus::tab_targets(ui, editor, tab.index) {
                    let command = GraphCommand::TabToGroup { from: tab.index, target };
                    commands.push(Command::Graph(command));
                }
            });
            menu_separator(ui);
            if menu_item(ui, None, None, "Rename", "").clicked() {
                ui_state.graph.renaming_tab = Some((tab.index, tab.name.clone()));
            }
            if menu_item_enabled(ui, Some("square-rounded-x"), Some(theme::ACCENT_RED), "Delete", "", !only_tab).clicked() {
                ui_state.graph.confirm_delete_tab = Some(tab.index);
            }
            menu_caption(ui, "Color");
            if let Some(swatch_name) = swatch_grid(ui) {
                commands.push(Command::SetGraphTabColor(tab.index, swatch_name));
                ui.close();
            }
        });

        let outline = tab_outline(tab_rect);
        if selected {
            let body = Rect::from_min_max(Pos2::new(tab_rect.left() + 4.0, tab_rect.top()), Pos2::new(tab_rect.right() - 4.0, tab_rect.bottom()));
            let corners = egui::CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 };
            ui.painter().rect_filled(body, corners, swatch.gamma_multiply(0.15));
            ui.painter().add(Shape::line(outline, Stroke::new(TAB_BORDER, swatch)));
        } else {
            if response.hovered() {
                ui.painter().rect_filled(tab_rect, 4.0, theme::GRAPH_TAB_BACKGROUND.gamma_multiply(0.15));
            }
            ui.painter().line_segment([Pos2::new(tab_rect.left(), line_y), Pos2::new(tab_rect.right(), line_y)], line_stroke);
        }
        let swatch_rect = Rect::from_min_max(Pos2::new(tab_rect.left() + 12.0, tab_rect.top() + 6.0), Pos2::new(tab_rect.left() + 15.0, tab_rect.bottom() - 6.0));
        ui.painter().rect_filled(swatch_rect, 1.5, swatch);

        let renaming = ui_state.graph.renaming_tab.as_ref().filter(|(index, _)| *index == tab.index).map(|(_, text)| text.clone());
        match renaming {
            Some(mut text) => {
                let edit_rect = Rect::from_min_max(Pos2::new(tab_rect.left() + 22.0, tab_rect.top() + 3.0), Pos2::new(tab_rect.right() - 6.0, tab_rect.bottom() - 3.0));
                let edit = egui::TextEdit::singleline(&mut text).font(theme::monospace(11.0));
                let edit_response = ui.put(edit_rect, edit);
                if !edit_response.has_focus() && !edit_response.lost_focus() {
                    edit_response.request_focus();
                }
                // An `EditablePopup`: a blank name is no name.
                match crate::ui::widgets::popup_outcome(ui, &edit_response) {
                    None => ui_state.graph.renaming_tab = Some((tab.index, text)),
                    Some(accepted) => {
                        if accepted && !text.trim().is_empty() {
                            commands.push(Command::RenameGraphTab(tab.index, text.trim().to_string()));
                        }
                        ui_state.graph.renaming_tab = None;
                    }
                }
            }
            None => {
                let text_position = Pos2::new(tab_rect.left() + TAB_NAME_LEFT, tab_rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(text_position, galley, theme::LABEL_EMPHASIS);
            }
        }
        // Tabs overlap by 1 (`Margin 0,0,-1,0`).
        x += width - 1.0;
    }
    ui.set_clip_rect(outer_clip);
    // `ExtraBorder`: the line on to the end of the strip, from the last
    // tab or, past their room, from its end.
    ui.painter().line_segment([Pos2::new(x.min(room.right()), line_y), Pos2::new(strip.right(), line_y)], line_stroke);
    if layout.overflow {
        let button = Rect::from_min_size(Pos2::new(room.right(), strip.top() + 1.0), Vec2::new(TAB_LIST_BUTTON, TAB_HEIGHT - TAB_BORDER));
        if let Some(index) = tab_list(ui, button, &tabs, ui_state.graph.tab) {
            ui_state.graph.tab = index;
        }
    }

    confirm_tab_delete(ui, &tabs, ui_state, commands);
    strip.left() + space + BOOKMARKS_GAP
}

/// The tab list button (`chevron-down`) after tabs that overflow the
/// strip: a menu of every tab, the shown one checked. Returns the tab
/// chosen.
fn tab_list(ui: &mut Ui, rect: Rect, tabs: &[GraphTabInfo], shown: usize) -> Option<usize> {
    let response = ui.interact(rect, ui.id().with("graph-tab-list"), Sense::click());
    let open = egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&response));
    if response.is_pointer_button_down_on() {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
    } else if response.hovered() || open {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    let color = if response.hovered() || open { theme::LABEL_EMPHASIS } else { theme::ICON_DEFAULT };
    icons::paint(ui, Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE)), "chevron-down", color);
    let response = response.tooltip("Graph Tabs");
    let mut chosen = None;
    let mut popup = egui::Popup::menu(&response);
    popup = popup.align(egui::RectAlign::BOTTOM_END);
    popup.show(|ui| {
        menu_rows(ui, |ui| {
            for tab in tabs {
                let swatch = theme::swatch_color(&tab.color);
                if checked_icon_item(ui, tab.index == shown, Some("square-rounded"), Some(swatch), &tab.name, "") {
                    chosen = Some(tab.index);
                }
            }
        });
    });
    chosen
}

/// The bookmarked nodes `stackBookmarks` shows, in the order they were
/// added; none while `HideBookmarkShortcuts` hides them.
fn bookmarked_nodes(editor: &Editor) -> Vec<&nerthus_engine::NodeData> {
    if editor.user.options.hide_bookmark_shortcuts() {
        return Vec::new();
    }
    editor.document.bookmarks().iter().filter_map(|id| editor.document.node(*id)).collect()
}

/// How much of the tab strip's end the bookmark bars take, `right_margin`
/// in, with the gap the tabs leave before them; nothing without any.
fn bookmarks_width(editor: &Editor, right_margin: f32) -> f32 {
    let count = bookmarked_nodes(editor).len();
    if count == 0 {
        return 0.0;
    }
    right_margin + count as f32 * (BOOKMARK_BUTTON.x + 2.0 * BOOKMARK_MARGIN) - BOOKMARK_MARGIN + BOOKMARKS_GAP
}

/// `stackBookmarks` (`UpdateBookmarkMenu`): a small bar for each bookmark
/// at the right of the tab strip, `right_margin` in, in its node's toolbox
/// colour, its name as the tooltip; a click goes to the node. They keep
/// right of `left`, where the tabs end: those with no room there stay off
/// the strip (the Bookmarks menu lists them all).
/// `HideBookmarkShortcuts` hides them.
fn bookmark_buttons(ui: &mut Ui, area: Rect, right_margin: f32, left: f32, editor: &Editor, commands: &mut Vec<Command>) {
    let mut right = area.right() - right_margin;
    let center_y = area.top() + TABS_HEIGHT / 2.0;
    for node in bookmarked_nodes(editor).iter().rev() {
        let rect = Rect::from_min_max(Pos2::new(right - BOOKMARK_MARGIN - BOOKMARK_BUTTON.x, center_y - BOOKMARK_BUTTON.y / 2.0), Pos2::new(right - BOOKMARK_MARGIN, center_y + BOOKMARK_BUTTON.y / 2.0));
        if rect.left() < left - 0.5 {
            break;
        }
        right = rect.left() - BOOKMARK_MARGIN;
        let response = ui.interact(rect, ui.id().with(("bookmark-button", node.id)), Sense::click());
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        let bar = rect.shrink(2.0);
        let color = graph_canvas::node_color(node);
        ui.painter().rect_filled(bar, 2.0, color);
        ui.painter().rect_stroke(bar, 2.0, Stroke::new(1.0, theme::BUTTON_BORDER_ALT), egui::StrokeKind::Inside);
        let response = response.tooltip(node.name.clone());
        if response.clicked() {
            commands.push(Command::GoToBookmark(node.id));
        }
    }
}

/// `MiDeleteGraphTab_Click`: asks before a tab and its nodes go.
fn confirm_tab_delete(ui: &mut Ui, tabs: &[GraphTabInfo], ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let Some(index) = ui_state.graph.confirm_delete_tab else {
        return;
    };
    let name = tabs.iter().find(|tab| tab.index == index).map(|tab| tab.name.clone()).unwrap_or_default();
    let modal = dialog(ui.ctx(), egui::Id::new("confirm-delete-tab"));
    let response = modal.show(ui.ctx(), |ui| {
        ui.set_width(320.0);
        ui.label(egui::RichText::new(format!("Delete Graph Tab \"{name}\"?")).font(theme::semibold(13.0)).color(theme::LABEL_EMPHASIS));
        ui.add_space(6.0);
        ui.label(format!("Are you sure you want to delete this Graph \"{name}\"?"));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if dialog_button(ui, "No", false).clicked() {
                ui_state.graph.confirm_delete_tab = None;
            }
            if dialog_button(ui, "Yes", false).clicked() {
                commands.push(Command::DeleteGraphTab(index));
                ui_state.graph.confirm_delete_tab = None;
            }
        });
    });
    if response.should_close() {
        ui_state.graph.confirm_delete_tab = None;
    }
}

/// The build strip's width while the graph has room for it, and its height.
const FOOTER_WIDTH: f32 = 200.0;
const FOOTER_HEIGHT: f32 = 26.0;
/// The build strip's inset from the graph's left and bottom edges (inside
/// the border), and from its right edge while the graph is narrower than
/// the strip.
const FOOTER_INSET: f32 = 1.75;
/// `ProgressX` in Lines mode: twenty ticks, 2 wide and 6 apart.
const FOOTER_TICKS: usize = 20;
const TICK_WIDTH: f32 = 2.0;
const TICK_SPACING: f32 = 6.0;
/// Room left after the last tick that fits.
const TICKS_END_MARGIN: f32 = 4.0;

/// The build strip at the bottom left of the graph `area`: 200 wide, or
/// narrower so it stays inside the graph's border when the graph is
/// narrow. None when the graph has no room for it at all.
fn footer_rect(area: Rect) -> Option<Rect> {
    let width = FOOTER_WIDTH.min(area.width() - 2.0 * FOOTER_INSET);
    if width <= 0.0 {
        return None;
    }
    let min = Pos2::new(area.left() + FOOTER_INSET, footer_top(area));
    Some(Rect::from_min_size(min, Vec2::new(width, FOOTER_HEIGHT)))
}

/// The top of the build strip at the bottom of the graph `area`.
fn footer_top(area: Rect) -> f32 {
    area.bottom() - FOOTER_INSET - FOOTER_HEIGHT
}

/// How many of the progress ticks, the first centred at `start_x`, fit
/// before `end_x`: all twenty in the full strip, fewer in a narrower one.
fn ticks_that_fit(start_x: f32, end_x: f32) -> usize {
    let room = end_x - TICK_WIDTH / 2.0 - start_x;
    if room < 0.0 {
        return 0;
    }
    ((room / TICK_SPACING).floor() as usize + 1).min(FOOTER_TICKS)
}

/// The build clock and progress (`StateIcon`, `TotalTime`, `Progress`),
/// kept to the strip `footer_rect` gives.
fn footer(ui: &mut Ui, area: Rect, editor: &Editor) {
    let Some(footer) = footer_rect(area) else {
        return;
    };
    let mut strip = ui.new_child(UiBuilder::new().max_rect(footer));
    strip.set_clip_rect(footer.intersect(ui.clip_rect()));
    let corners = egui::CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 };
    strip.painter().rect_filled(footer, corners, theme::PANEL_CLADDING);
    let icon_color = if editor.building { theme::ACCENT_PRIMARY } else { theme::ICON_DEFAULT };
    let icon_rect = Rect::from_min_size(footer.min + Vec2::new(8.0, 5.0), Vec2::splat(16.0));
    icons::paint(&strip, icon_rect, "activity-heartbeat", icon_color);

    let seconds = editor.build_seconds();
    let minutes = (seconds / 60.0).floor() as u32;
    let remainder = (seconds % 60.0).floor() as u32;
    let clock = format!("{minutes:02}:{remainder:02}");
    let galley = strip.painter().layout_no_wrap(clock, theme::monospace_semibold(9.0), theme::LABEL_DEFAULT);
    let clock_position = Pos2::new(icon_rect.right() + 4.0, footer.center().y - galley.size().y / 2.0);
    let clock_width = galley.size().x;
    strip.painter().galley(clock_position, galley, theme::LABEL_DEFAULT);

    let separator_x = clock_position.x + clock_width + 8.0;
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR);
    strip.painter().line_segment([Pos2::new(separator_x, footer.center().y - 4.0), Pos2::new(separator_x, footer.center().y + 4.0)], stroke);

    // `ProgressX` in Lines mode: the ticks that fit, lit by progress.
    let start_x = separator_x + 8.0;
    let ticks = ticks_that_fit(start_x, footer.right() - TICKS_END_MARGIN);
    let progress = if editor.building { editor.build_progress } else { 0.0 };
    let lit = (progress * ticks as f32).round() as usize;
    for tick in 0..ticks {
        let tick_x = start_x + tick as f32 * TICK_SPACING;
        let color = if tick < lit { theme::ACCENT_PRIMARY } else { theme::PROGRESS_EMPTY };
        let top = Pos2::new(tick_x, footer.top() + 8.0);
        let bottom = Pos2::new(tick_x, footer.bottom() - 8.0);
        strip.painter().line_segment([top, bottom], Stroke::new(TICK_WIDTH, color));
    }
    if editor.building {
        ui.ctx().request_repaint();
    }
}

/// The crawling red outline while the engine is suspended (`EngineRect`),
/// `top` down.
fn engine_suspended_outline(ui: &Ui, area: Rect, top: f32) {
    let rect = Rect::from_min_max(area.min + Vec2::new(4.0, top), area.max - Vec2::splat(4.0));
    let points = vec![rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()];
    let time = ui.input(|input| input.time) as f32;
    let offset = (time * 16.0) % 6.0;
    let stroke = Stroke::new(1.5, theme::ACCENT_RED);
    let dashes = Shape::dashed_line_with_offset(&points, stroke, &[1.5], &[4.5], offset);
    ui.painter().extend(dashes);
    ui.ctx().request_repaint();
}

/// The toolbar right of the graph (`GraphToolbarBorder`, `Menu-Border`
/// 1.5): its corners square on the left while the Data View shows beside
/// it (`data_shown`).
fn graph_toolbar(ui: &mut Ui, toolbar_rect: Rect, data_shown: bool, editor: &Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    let corners = if data_shown { CornerRadius { nw: 0, ne: 4, sw: 0, se: 4 } } else { CornerRadius::same(4) };
    toolbar_background_corners(ui, toolbar_rect, corners, theme::MENU_BORDER, 1.5);
    let [padding_across, padding_down] = theme::sizes(ui.ctx()).toolbar_padding;
    let inner = toolbar_rect.shrink(1.5).shrink2(Vec2::new(padding_across, padding_down));

    vertical_toolbar(ui, "graph-toolbar", inner, toolbar_rect, |column, dock| {
        // `btnExpandToggle`. While the graph leaves the open Data View no
        // room it shows shut, and the chevron can't open it until there is.
        if ui_state.data_view.expanded && !data_shown {
            column.add_enabled_ui(false, |ui| {
                let tip = "Data View: no room beside the graph (widen the window or the graph area)";
                icon_button(ui, "chevron-left", "Data View", None).disabled_tooltip(tip);
            });
        } else {
            let chevron = if data_shown { "chevron-right" } else { "chevron-left" };
            if icon_button(column, chevron, "Data View", None).clicked() {
                ui_state.data_view.toggle();
            }
        }
        toolbar_separator(column, 8.0, 8.0);
        icon_menu(column, "bookmarks", "Node Bookmarks", None, |ui| {
            menus::bookmarks_menu(ui, editor, ui_state, commands);
        });
        icon_menu(column, "tournament", "Node Commands", None, |ui| {
            if menu_item(ui, Some("arrow-left-rhombus"), Some(theme::ACCENT_MUTED_PURPLE), "Select upstream", "").clicked() {
                commands.push(Command::SelectUpstream);
            }
            if menu_item(ui, Some("arrow-right-rhombus"), Some(theme::ACCENT_MUTED_PURPLE), "Select downstream", "").clicked() {
                commands.push(Command::SelectDownstream);
            }
            menu_separator(ui);
            if menu_item(ui, Some("layers-linked"), None, "Group / Ungroup", "").clicked() {
                commands.push(Command::Graph(GraphCommand::GroupUngroup));
            }
            if menu_item(ui, Some("arrows-join-2"), Some(theme::ACCENT_MUTED_ORANGE), "Combine nodes", "").clicked() {
                commands.push(Command::Graph(GraphCommand::Combine(CombineKind::Toolbar)));
            }
            menu_separator(ui);
            submenu(ui, Some("keyframe"), None, "Convert port to portals", |ui| {
                menus::convert_port_items(ui, editor, commands);
            });
            menu_separator(ui);
            submenu(ui, None, None, "Move to Tab", |ui| {
                if let Some(target) = menus::tab_targets(ui, editor, ui_state.graph.tab) {
                    commands.push(Command::Graph(GraphCommand::MoveToTab(target)));
                }
            });
        });
        toolbar_separator(column, 8.0, 8.0);
        icon_menu(column, "flame", "Bake", None, |ui| {
            menus::toolbar_bake_items(ui, editor, commands);
        });

        // Docked at the bottom, down from the top: Refresh and Mutate (each
        // with its right-click menu: `btnRefresh`, `btnMutate`), the engine
        // toggle (`Accent-Muted-Red`; `Accent-Red` outline while suspended)
        // and Graph Commands, with separators (margin 2, 8) between. A
        // separator's margin at least keeps them from Bake.
        column.add_space(8.0);
        dock.bottom(column);
        let refresh = icon_button(column, "rotate-360", "Refresh", None);
        if refresh.clicked() {
            commands.push(Command::Refresh);
        }
        refresh.context_menu(|ui| {
            menu_style(ui);
            if menu_item(ui, Some("rotate-360"), None, "Refresh all nodes", "").clicked() {
                commands.push(Command::RefreshAll);
            }
        });
        let mutate = icon_button(column, "potion", "Mutate", None);
        if mutate.clicked() {
            commands.push(Command::Graph(GraphCommand::MutateButton));
        }
        mutate.context_menu(|ui| {
            menu_style(ui);
            if menu_item(ui, Some("potion"), None, "Mutate all nodes", "").clicked() {
                commands.push(Command::Graph(GraphCommand::MutateAll));
            }
        });
        toolbar_separator(column, 8.0, 8.0);
        let engine_tip = if editor.engine_active { "Suspend Engine (Currently running)" } else { "Resume Engine (Currently suspended)" };
        if engine_toggle(column, !editor.engine_active).tooltip(engine_tip).clicked() {
            commands.push(Command::ToggleEngine);
        }
        toolbar_separator(column, 8.0, 8.0);
        icon_menu(column, "menu-deep", "Graph Commands", None, |ui| {
            menus::graph_commands(ui, editor, ui_state, commands);
        });
    });
}

/// `tglEngine`: the engine icon in `Accent-Muted-Red`; checked (the
/// engine suspended), the pressed fill with an `Accent-Red` outline.
fn engine_toggle(ui: &mut Ui, suspended: bool) -> egui::Response {
    let size = Vec2::splat(crate::ui::widgets::toolbar_button(ui));
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hovered = response.hovered();
    if suspended && !hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_PRESSED);
    } else if hovered {
        ui.painter().rect_filled(rect, 4.0, theme::BUTTON_HOVER);
    }
    if suspended {
        ui.painter().rect_stroke(rect, 4.0, Stroke::new(1.5, theme::ACCENT_RED), egui::StrokeKind::Inside);
    }
    let color = if hovered || suspended { theme::LABEL_EMPHASIS } else { theme::ACCENT_MUTED_RED };
    let icon_rect = Rect::from_center_size(rect.center(), Vec2::splat(icons::DEFAULT_SIZE));
    icons::paint(ui, icon_rect, "engine", color);
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Document;
    use crate::user::ToolboxMode;

    /// The build strip painted at the bottom of a graph frame `width` wide,
    /// its clock at `seconds`: the strip and the shapes painted, and among
    /// them the progress ticks (the 2-wide lines).
    fn painted_footer(width: f32, seconds: f32) -> (Rect, Rect, Vec<egui::epaint::ClippedShape>, Vec<[Pos2; 2]>) {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut editor = Editor::new(Document::new());
        editor.last_build_seconds = seconds;
        let area = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 400.0));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 960.0))), ..Default::default() };
        let mut output = context.run_ui(input, |ui| footer(ui, area, &editor));
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        let ticks = shapes.iter().filter_map(|clipped| match clipped.shape {
            Shape::LineSegment { points, stroke } if stroke.width == TICK_WIDTH => Some(points),
            _ => None,
        });
        let ticks = ticks.collect();
        (area, footer_rect(area).unwrap_or(Rect::NOTHING), shapes, ticks)
    }

    #[test]
    fn a_wide_graph_shows_the_whole_strip() {
        for seconds in [0.0, 99.0 * 60.0 + 59.0, 100.0 * 60.0] {
            let (area, strip, _, ticks) = painted_footer(800.0, seconds);
            assert_eq!(strip.width(), FOOTER_WIDTH);
            assert_eq!(strip.left(), area.left() + FOOTER_INSET);
            assert_eq!(strip.bottom(), area.bottom() - FOOTER_INSET);
            assert_eq!(ticks.len(), FOOTER_TICKS);
            assert!(ticks.iter().all(|points| points[0].x + TICK_WIDTH / 2.0 <= strip.right()));
        }
    }

    #[test]
    fn a_narrow_graph_keeps_the_strip_inside_its_border() {
        // The graph frame beside the Data View at `MIN_GRAPH` (less its
        // margins and the grid's 1 on the right), and narrower still when
        // the window leaves the Data View its 340 and no more.
        let at_min_graph = MIN_GRAPH - 2.0 * AREA_MARGIN - 1.0;
        for width in [at_min_graph, 120.0, 80.0, 40.0] {
            let (area, strip, shapes, ticks) = painted_footer(width, 0.0);
            let inside_border = area.right() - TAB_BORDER;
            assert!(strip.right() <= inside_border, "{width}: the strip ends at {} past {inside_border}", strip.right());
            assert!(ticks.len() < FOOTER_TICKS, "{width}: all {} ticks drawn", ticks.len());
            for points in &ticks {
                assert!(points[0].x + TICK_WIDTH / 2.0 <= strip.right() - TICKS_END_MARGIN, "{width}: a tick at {} past the strip", points[0].x);
            }
            for clipped in &shapes {
                assert!(strip.contains_rect(clipped.clip_rect), "{width}: {:?} painted outside the strip", clipped.shape);
            }
        }
        let (_, _, _, ticks) = painted_footer(at_min_graph, 0.0);
        assert!(!ticks.is_empty(), "the strip at the graph's minimum width still shows progress");
    }

    #[test]
    fn a_graph_with_no_room_paints_no_strip() {
        let (_, _, shapes, _) = painted_footer(2.0 * FOOTER_INSET, 0.0);
        assert!(shapes.is_empty());
    }

    /// One frame of the graph grid in `grid` on a 1600 by 960 screen, with
    /// `events`: the shapes painted and the commands asked for.
    fn grid_frame(context: &egui::Context, grid: Rect, editor: &mut Editor, ui_state: &mut UiState, events: Vec<egui::Event>) -> (Vec<egui::epaint::ClippedShape>, Vec<Command>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 960.0))), events, ..Default::default() };
        let mut commands = Vec::new();
        let mut output = context.run_ui(input, |ui| show(ui, grid, editor, ui_state, &mut commands));
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        (shapes, commands)
    }

    /// A click at `at` over three frames: onto it, pressed, released.
    fn click(context: &egui::Context, grid: Rect, editor: &mut Editor, ui_state: &mut UiState, at: Pos2) -> Vec<Command> {
        let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        grid_frame(context, grid, editor, ui_state, vec![egui::Event::PointerMoved(at)]);
        grid_frame(context, grid, editor, ui_state, vec![button(true)]);
        grid_frame(context, grid, editor, ui_state, vec![button(false)]).1
    }

    fn graph_grid_context() -> (egui::Context, Editor, UiState) {
        let context = egui::Context::default();
        theme::apply(&context);
        (context, Editor::new(Document::new()), UiState::default())
    }

    /// The Data View's least width, as `show` measures it.
    fn least_width(context: &egui::Context) -> f32 {
        let mut least = 0.0;
        let output = context.run_ui(egui::RawInput::default(), |ui| least = data_view::least_width(ui));
        output.drop_without_applying_deltas();
        least
    }

    /// The toolbar column's width while the Data View is shut.
    fn shut_toolbar_column(context: &egui::Context) -> f32 {
        AREA_MARGIN + toolbar_width(theme::sizes(context)) + AREA_MARGIN + viewport_area::TOOLBAR_MARGIN
    }

    /// Where the Data View chevron, the toolbar's first button, sits.
    fn chevron_at(context: &egui::Context, grid: Rect) -> Pos2 {
        let sizes = theme::sizes(context);
        let toolbar_right = grid.right() - AREA_MARGIN - viewport_area::TOOLBAR_MARGIN;
        let x = toolbar_right - toolbar_width(sizes) / 2.0;
        let y = grid.top() + AREA_MARGIN + 1.5 + sizes.toolbar_padding[1] + sizes.toolbar_button / 2.0;
        Pos2::new(x, y)
    }

    /// The graph frame's edges round the canvas: the area's margins, the
    /// grid's 1 on the right and the border with its margin of 2.
    const CANVAS_INSET: f32 = 2.0 * AREA_MARGIN + 1.0 + 2.0 * (TAB_BORDER + 2.0);

    #[test]
    fn the_data_view_gives_up_width_to_the_graph() {
        let least = 240.0;
        let share = data_view::DEFAULT_SHARE;
        // Room for both: its share, at least 340.
        assert_eq!(data_view_width(1000.0, share, least), data_view::MIN_WIDTH);
        assert_eq!(data_view_width(3000.0, share, least), 3000.0 * share);
        assert_eq!(data_view_width(800.0, 0.9, least), 800.0 - MIN_GRAPH);
        // Less room than 340 and the graph's minimum: it narrows.
        assert_eq!(data_view_width(450.0, share, least), 450.0 - MIN_GRAPH);
        assert_eq!(data_view_width(MIN_GRAPH + least, share, least), least);
        // Less than its least: it steps aside.
        for stars in [MIN_GRAPH + least - 1.0, 300.0, MIN_GRAPH, 100.0, 0.0] {
            assert_eq!(data_view_width(stars, share, least), 0.0, "{stars}");
        }
        // A least wider than 340 holds too.
        assert_eq!(data_view_width(1000.0, share, 360.0), 360.0);
        for stars in (0..2000).map(|stars| stars as f32) {
            for share in [0.0, share, 0.5, 1.0] {
                let width = data_view_width(stars, share, least);
                assert!(width == 0.0 || (width >= least && stars - width >= MIN_GRAPH), "{stars} at {share}: {width}");
            }
        }
    }

    #[test]
    fn the_graph_keeps_its_minimum_beside_the_open_data_view() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        ui_state.data_view.expanded = true;
        let least = least_width(&context);
        let mut narrowed = false;
        let mut stepped_aside = false;
        for width in (250..=1400).step_by(10) {
            let grid = Rect::from_min_size(Pos2::new(4.0, 300.0), Vec2::new(width as f32, 400.0));
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
            let canvas = ui_state.graph.rect;
            assert!(canvas.width() + CANVAS_INSET >= MIN_GRAPH - 0.5, "{width}: the graph is {} wide", canvas.width());
            match ui_state.graph.areas.data_view {
                Some(data) => {
                    assert!(data.left() >= canvas.right(), "{width}: the Data View at {} over the graph to {}", data.left(), canvas.right());
                    assert!(data.width() >= least - 0.5, "{width}: the Data View is {} wide, its tabs {least}", data.width());
                    narrowed |= data.width() + AREA_MARGIN < data_view::MIN_WIDTH - 0.5;
                }
                None => {
                    // The graph reaches the toolbar.
                    let reach = grid.right() - shut_toolbar_column(&context) - AREA_MARGIN - 1.0 - TAB_BORDER - 2.0;
                    assert!((canvas.right() - reach).abs() <= 0.5, "{width}: the graph ends at {} short of {reach}", canvas.right());
                    stepped_aside = true;
                }
            }
            if width >= 1000 {
                let data = ui_state.graph.areas.data_view.expect("room for the Data View");
                assert!(data.width() + AREA_MARGIN >= data_view::MIN_WIDTH - 0.5, "{width}: the Data View is {} wide", data.width());
            }
        }
        assert!(narrowed && stepped_aside);
        assert!(ui_state.data_view.expanded, "stepping aside leaves the Data View open for when the room is back");
    }

    #[test]
    fn what_the_graph_draws_keeps_to_its_column() {
        // A graph narrower than its tab and shorter than its toolbox, the
        // toolbar shorter than its buttons.
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(150.0, 140.0));
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let area_right = grid.right() - shut_toolbar_column(&context) - AREA_MARGIN;
        for clipped in &shapes {
            let visible = clipped.shape.visual_bounding_rect().intersect(clipped.clip_rect);
            if !visible.is_positive() {
                continue;
            }
            assert!(grid.expand(0.5).contains_rect(visible), "{:?} painted at {visible:?} outside the grid {grid:?}", clipped.shape);
            if visible.left() < area_right {
                assert!(visible.right() <= area_right + 0.5, "{:?} painted at {visible:?} past the graph's edge {area_right}", clipped.shape);
            }
        }
    }

    /// The toolbox's category buttons as painted: the part of each outline
    /// (`TOOLBOX_BUTTON_BACKGROUND`) left in sight by its clip.
    fn toolbox_buttons(shapes: &[egui::epaint::ClippedShape]) -> Vec<Rect> {
        let buttons = shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Rect(rect) if rect.stroke.color == theme::TOOLBOX_BUTTON_BACKGROUND => Some(rect.rect.intersect(clipped.clip_rect)),
            _ => None,
        });
        buttons.filter(|visible| visible.is_positive()).collect()
    }

    /// Where the toolbox shows `text` (an Expanded category's name), in
    /// full sight.
    fn toolbox_text(shapes: &[egui::epaint::ClippedShape], text: &str) -> Option<Rect> {
        shapes.iter().find_map(|clipped| match &clipped.shape {
            Shape::Text(shape) if shape.galley.text() == text => {
                let bounds = clipped.shape.visual_bounding_rect();
                clipped.clip_rect.contains_rect(bounds).then_some(bounds)
            }
            _ => None,
        })
    }

    /// The graph frame's inside: `grid` less its border (and the tabs).
    fn graph_frame(grid: Rect, tabs_hidden: bool) -> Rect {
        let top = if tabs_hidden { grid.top() } else { grid.top() + GRAPH_TOP };
        Rect::from_min_max(Pos2::new(grid.left(), top), grid.max).shrink(TAB_BORDER)
    }

    #[test]
    fn the_toolbox_menus_keep_inside_the_graph_above_the_build_strip() {
        for mode in [ToolboxMode::Single, ToolboxMode::Double, ToolboxMode::Expanded] {
            for tabs_hidden in [false, true] {
                let (context, mut editor, mut ui_state) = graph_grid_context();
                editor.user.options.set_toolbox_style(ToolboxStyle::Menus);
                editor.user.options.set_toolbox_mode(mode);
                editor.user.options.set_value("HideTabs", serde_json::json!(tabs_hidden));
                let mut scrolled = false;
                for height in (60..=700).step_by(20) {
                    let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, height as f32));
                    grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                    let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                    let case = format!("{mode:?} at {height} (tabs hidden: {tabs_hidden})");
                    let toolbox = ui_state.graph.toolbox_rect;
                    let strip_top = footer_top(grid);
                    if toolbox.height() > 0.0 {
                        assert!(toolbox.bottom() <= strip_top, "{case}: the toolbox reaches {} under the strip at {strip_top}", toolbox.bottom());
                    }
                    let frame = graph_frame(grid, tabs_hidden);
                    let buttons = toolbox_buttons(&shapes);
                    for button in &buttons {
                        assert!(toolbox.expand(0.5).contains_rect(*button), "{case}: a button at {button:?} past the toolbox {toolbox:?}");
                        assert!(frame.contains_rect(*button), "{case}: a button at {button:?} outside the graph frame {frame:?}");
                        assert!(button.bottom() <= strip_top, "{case}: a button at {button:?} under the strip at {strip_top}");
                    }
                    scrolled |= buttons.len() < toolbox::CATEGORIES.len();
                    if height >= 600 {
                        assert_eq!(buttons.len(), toolbox::CATEGORIES.len(), "{case}: not every category shows");
                    }
                }
                assert!(scrolled, "{mode:?}: no graph short enough to cut the toolbox off");
            }
        }
    }

    #[test]
    fn every_toolbox_category_is_in_reach_in_a_short_graph() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        editor.user.options.set_toolbox_style(ToolboxStyle::Menus);
        editor.user.options.set_toolbox_mode(ToolboxMode::Expanded);
        // About the graph of a 1280 by 500 window: three categories show.
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 200.0));
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        assert!(toolbox_text(&shapes, "Primitive").is_some());
        assert!(toolbox_text(&shapes, "Macro").is_none(), "Macro shows without scrolling");
        // The wheel over the toolbox scrolls it, not the graph.
        let toolbox = ui_state.graph.toolbox_rect;
        let zoom = ui_state.graph.view().zoom;
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
        grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(toolbox.center())]);
        grid_frame(&context, grid, &mut editor, &mut ui_state, vec![wheel]);
        for _ in 0..60 {
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        }
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        assert_eq!(ui_state.graph.view().zoom, zoom);
        assert!(toolbox_text(&shapes, "Primitive").is_none(), "the toolbox did not scroll");
        let name = toolbox_text(&shapes, "Macro").expect("Macro in sight at the end");
        assert!(toolbox.contains_rect(name), "Macro at {name:?} outside the toolbox {toolbox:?}");
        // Its menu opens from there.
        assert!(!egui::Popup::is_any_open(&context));
        click(&context, grid, &mut editor, &mut ui_state, name.center());
        assert!(egui::Popup::is_any_open(&context), "Macro's menu did not open");
    }

    #[test]
    fn a_click_under_the_toolbox_opens_no_menu() {
        // Below the cut-off column, on the strip: no category's menu.
        let (context, mut editor, mut ui_state) = graph_grid_context();
        editor.user.options.set_toolbox_style(ToolboxStyle::Menus);
        editor.user.options.set_toolbox_mode(ToolboxMode::Single);
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 200.0));
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let toolbox = ui_state.graph.toolbox_rect;
        for y in [toolbox.bottom() + 4.0, footer_top(grid) + 4.0, grid.bottom() + 4.0] {
            click(&context, grid, &mut editor, &mut ui_state, Pos2::new(toolbox.center().x, y));
            assert!(!egui::Popup::is_any_open(&context), "a menu opened at {y}");
        }
    }

    #[test]
    fn a_short_toolbar_scrolls_to_its_bottom_group_and_keeps_it_inside() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let grid = Rect::from_min_size(Pos2::new(10.0, 300.0), Vec2::new(900.0, 250.0));
        let chevron = chevron_at(&context, grid);
        // Above the toolbar, where the viewport toolbar is, nothing of the
        // graph's takes a click.
        for y in (240..300).step_by(6) {
            let commands = click(&context, grid, &mut editor, &mut ui_state, Pos2::new(chevron.x, y as f32));
            assert!(commands.is_empty(), "a click at {y} above the toolbar: {commands:?}");
        }
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
        grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(chevron)]);
        grid_frame(&context, grid, &mut editor, &mut ui_state, vec![wheel]);
        for _ in 0..60 {
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        }
        // Scrolled to the end: Graph Commands at the foot, the engine
        // toggle and Mutate over it with a separator (17) between each,
        // and Refresh on top.
        let sizes = theme::sizes(&context);
        let foot = grid.bottom() - AREA_MARGIN - 1.5 - sizes.toolbar_padding[1];
        let refresh = Pos2::new(chevron.x, foot - 3.5 * sizes.toolbar_button - 2.0 * 17.0);
        let commands = click(&context, grid, &mut editor, &mut ui_state, refresh);
        assert!(commands.iter().any(|command| matches!(command, Command::Refresh)), "{commands:?}");
        let mutate = refresh + Vec2::new(0.0, sizes.toolbar_button);
        let commands = click(&context, grid, &mut editor, &mut ui_state, mutate);
        assert!(commands.iter().any(|command| matches!(command, Command::Graph(GraphCommand::MutateButton))), "{commands:?}");
    }

    #[test]
    fn a_short_toolbar_keeps_the_data_view_chevron_clickable() {
        // Too short for both groups: the bottom one (Refresh at its top)
        // follows the top one, out of sight until the toolbar scrolls.
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 150.0));
        let commands = click(&context, grid, &mut editor, &mut ui_state, chevron_at(&context, grid));
        assert!(!commands.iter().any(|command| matches!(command, Command::Refresh)));
        assert!(ui_state.data_view.expanded);
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        assert!(ui_state.graph.areas.data_view.is_some());
    }

    #[test]
    fn the_chevron_waits_for_room_for_the_data_view() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        ui_state.data_view.expanded = true;
        let narrow = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(300.0, 500.0));
        click(&context, narrow, &mut editor, &mut ui_state, chevron_at(&context, narrow));
        assert!(ui_state.data_view.expanded);
        assert_eq!(ui_state.graph.areas.data_view, None);
        let wide = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 500.0));
        grid_frame(&context, wide, &mut editor, &mut ui_state, Vec::new());
        assert!(ui_state.graph.areas.data_view.is_some());
        click(&context, wide, &mut editor, &mut ui_state, chevron_at(&context, wide));
        assert!(!ui_state.data_view.expanded);
    }

    #[test]
    fn tabs_narrow_before_they_scroll() {
        // Room for every tab: each as it is.
        assert_eq!(tab_layout(&[TAB_WIDTH, 200.0], 400.0), TabLayout { widths: vec![TAB_WIDTH, 200.0], room: 400.0, overflow: false });
        // A long name gives way first, down to the others' width.
        let layout = tab_layout(&[TAB_WIDTH, 460.0, TAB_WIDTH], 500.0);
        assert_eq!(layout.widths, [TAB_WIDTH, 232.0, TAB_WIDTH]);
        assert!(!layout.overflow);
        // Then every tab alike.
        assert_eq!(tab_layout(&[TAB_WIDTH; 6], 600.0).widths, [100.0; 6]);
        // Short of their least, they scroll with the tab list after them.
        let layout = tab_layout(&[TAB_WIDTH; 6], 400.0);
        assert_eq!(layout, TabLayout { widths: vec![TAB_MIN_WIDTH; 6], room: 400.0 - TAB_LIST_BUTTON, overflow: true });
        // A lone tab narrows to the strip.
        assert_eq!(tab_layout(&[460.0], 200.0), TabLayout { widths: vec![200.0], room: 200.0, overflow: false });

        for count in 1..12 {
            let natural: Vec<f32> = (0..count).map(|tab| TAB_WIDTH + 37.0 * (tab % 4) as f32).collect();
            for space in (7..1800).step_by(7).map(|space| space as f32) {
                let layout = tab_layout(&natural, space);
                let (_, length) = tab_spans(&layout.widths);
                let case = format!("{count} tabs in {space}");
                assert_eq!(layout.widths.len(), count, "{case}");
                for (width, wanted) in layout.widths.iter().zip(&natural) {
                    assert!(width <= wanted, "{case}: a tab {width} wide wants {wanted}");
                    if count > 1 {
                        assert!(*width >= TAB_MIN_WIDTH, "{case}: a tab {width} wide");
                    }
                }
                if layout.overflow {
                    assert!(count > 1 && length > layout.room, "{case}: overflows with {length} in {}", layout.room);
                    assert_eq!(layout.room, (space - TAB_LIST_BUTTON).max(0.0), "{case}");
                } else {
                    assert!(length <= space, "{case}: {length} long");
                    assert_eq!(layout.room, space, "{case}");
                }
                if tab_spans(&natural).1 <= space {
                    assert_eq!(layout.widths, natural, "{case}: narrowed with room to spare");
                }
            }
        }
    }

    #[test]
    fn a_tab_is_scrolled_into_sight_as_little_as_it_takes() {
        assert_eq!(scroll_to_show(0.0, (0.0, 100.0), 300.0), 0.0);
        assert_eq!(scroll_to_show(0.0, (400.0, 500.0), 300.0), 200.0);
        assert_eq!(scroll_to_show(250.0, (100.0, 200.0), 300.0), 100.0);
        assert_eq!(scroll_to_show(150.0, (200.0, 400.0), 300.0), 150.0);
        // Wider than the room: its start.
        assert_eq!(scroll_to_show(0.0, (100.0, 500.0), 300.0), 100.0);
    }

    /// The tab strip of the graph grid in `grid` with the Data View shut:
    /// the top of the graph area, less the grid's 1 on the right.
    fn tab_strip(context: &egui::Context, grid: Rect) -> Rect {
        let right = grid.right() - shut_toolbar_column(context) - AREA_MARGIN - 1.0;
        Rect::from_min_max(grid.min + Vec2::splat(AREA_MARGIN), Pos2::new(right, grid.top() + AREA_MARGIN + TABS_HEIGHT))
    }

    /// The tab names painted in `strip`: each name (whole), where it is
    /// drawn, its clip and whether it was cut short.
    fn tab_names(shapes: &[egui::epaint::ClippedShape], strip: Rect) -> Vec<(String, Rect, Rect, bool)> {
        let names = shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Text(text) if strip.contains(clipped.shape.visual_bounding_rect().center()) => {
                Some((text.galley.text().to_string(), clipped.shape.visual_bounding_rect(), clipped.clip_rect, text.galley.elided))
            }
            _ => None,
        });
        names.collect()
    }

    /// The tab named `name` as painted, when it is.
    fn tab_name<'a>(names: &'a [(String, Rect, Rect, bool)], name: &str) -> Option<&'a (String, Rect, Rect, bool)> {
        names.iter().find(|(shown, ..)| shown == name)
    }

    /// Whether the painted name is in full sight.
    fn in_full_sight(name: &(String, Rect, Rect, bool)) -> bool {
        name.2.contains_rect(name.1)
    }

    /// The bookmark bars painted (6 by 12, outlined).
    fn bookmark_bars(shapes: &[egui::epaint::ClippedShape]) -> Vec<Rect> {
        let bars = shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Rect(rect) if rect.stroke.color == theme::BUTTON_BORDER_ALT && (rect.rect.size() - Vec2::new(6.0, 12.0)).length() < 0.01 => Some(rect.rect.intersect(clipped.clip_rect)),
            _ => None,
        });
        bars.filter(|bar| bar.is_positive()).collect()
    }

    /// Six tabs, Graph 1 to Graph 6.
    fn six_tabs(editor: &mut Editor) -> Vec<GraphTabInfo> {
        for _ in 0..5 {
            editor.document.add_graph_tab();
        }
        editor.document.graph_tabs()
    }

    #[test]
    fn the_shown_tab_stays_in_sight_and_the_tabs_in_their_room() {
        // About the graph of a 640 by 480 window, and wider.
        for width in [300.0, 480.0, 640.0, 900.0] {
            let (context, mut editor, mut ui_state) = graph_grid_context();
            let tabs = six_tabs(&mut editor);
            let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 300.0));
            let strip = tab_strip(&context, grid);
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
            for tab in tabs.iter().chain(tabs.iter().rev()) {
                ui_state.graph.tab = tab.index;
                grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                let case = format!("{} shown at {width}", tab.name);
                let names = tab_names(&shapes, strip);
                let shown = tab_name(&names, &tab.name).unwrap_or_else(|| panic!("{case}: its name is not painted"));
                assert!(in_full_sight(shown), "{case}: its name at {:?} is cut by {:?}", shown.1, shown.2);
                // Nothing of the tabs past their room: before the tab list
                // while it shows.
                let overflow = tab_layout(&[TAB_WIDTH; 6], strip.width()).overflow;
                let room_right = if overflow { strip.right() - TAB_LIST_BUTTON } else { strip.right() };
                let band = Rect::from_min_max(strip.min, Pos2::new(strip.right(), strip.top() + 14.0));
                let list_button = Rect::from_min_max(Pos2::new(room_right, strip.top()), strip.max);
                for clipped in &shapes {
                    let visible = clipped.shape.visual_bounding_rect().intersect(clipped.clip_rect);
                    // Not the graph's background, nor the tab list's icon.
                    let icon = overflow && list_button.contains_rect(visible);
                    if !visible.intersects(band) || visible.contains_rect(band) || icon {
                        continue;
                    }
                    assert!(visible.left() >= strip.left() - 0.5 && visible.right() <= room_right + 0.5, "{case}: {:?} painted at {visible:?} past the tabs' room to {room_right}", clipped.shape);
                }
            }
        }
    }

    #[test]
    fn the_bookmark_bars_keep_their_own_room() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let tabs = six_tabs(&mut editor);
        let mountain = editor.document.add_node("Mountain", [25100.0, 25100.0]).expect("a Mountain");
        let erosion = editor.document.add_node("ErosionPro", [25400.0, 25100.0]).expect("an Erosion");
        editor.document.add_bookmark(mountain);
        editor.document.add_bookmark(erosion);
        let first = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(400.0, 300.0));
        grid_frame(&context, first, &mut editor, &mut ui_state, Vec::new());
        // From six tabs with room to spare, through ones that just fit, to
        // ones that scroll.
        for width in (400..=1300).step_by(25) {
            let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width as f32, 300.0));
            let strip = tab_strip(&context, grid);
            for tab in [&tabs[0], &tabs[5]] {
                ui_state.graph.tab = tab.index;
                grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                let case = format!("{} shown at {width}", tab.name);
                let bars = bookmark_bars(&shapes);
                assert_eq!(bars.len(), 2, "{case}: the bookmark bars");
                let bars_left = bars.iter().map(|bar| bar.left()).fold(f32::INFINITY, f32::min);
                for (name, bounds, clip, _) in tab_names(&shapes, strip) {
                    let visible = bounds.intersect(clip);
                    if visible.is_positive() {
                        assert!(visible.right() < bars_left - BOOKMARK_MARGIN, "{case}: {name} at {visible:?} under the bookmarks from {bars_left}");
                    }
                }
            }
        }
    }

    #[test]
    fn the_tab_list_reaches_every_tab() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let tabs = six_tabs(&mut editor);
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(480.0, 400.0));
        let strip = tab_strip(&context, grid);
        // The first frame opens the project on its first tab.
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        ui_state.graph.tab = tabs[5].index;
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let names = tab_names(&shapes, strip);
        assert!(!tab_name(&names, "Graph 1").is_some_and(in_full_sight), "Graph 1 shows beside Graph 6");

        let button = Pos2::new(strip.right() - TAB_LIST_BUTTON / 2.0, strip.top() + 1.0 + TAB_HEIGHT / 2.0);
        click(&context, grid, &mut editor, &mut ui_state, button);
        assert!(egui::Popup::is_any_open(&context), "the tab list did not open");
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let below = Rect::from_min_max(Pos2::new(grid.left(), strip.bottom()), grid.max);
        let listed = tab_names(&shapes, below);
        for tab in &tabs {
            assert!(tab_name(&listed, &tab.name).is_some(), "{} is not in the tab list", tab.name);
        }
        let first = tab_name(&listed, "Graph 1").expect("Graph 1 in the list").1.center();
        click(&context, grid, &mut editor, &mut ui_state, first);
        assert_eq!(ui_state.graph.tab, tabs[0].index);
        assert!(!egui::Popup::is_any_open(&context));
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let names = tab_names(&shapes, strip);
        assert!(tab_name(&names, "Graph 1").is_some_and(in_full_sight), "Graph 1 is not brought into sight");
    }

    #[test]
    fn the_wheel_scrolls_the_tabs() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        six_tabs(&mut editor);
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(480.0, 400.0));
        let strip = tab_strip(&context, grid);
        let zoom = ui_state.graph.view().zoom;
        let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: Vec2::new(0.0, -10_000.0), phase: egui::TouchPhase::Move, modifiers: egui::Modifiers::NONE };
        grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(strip.center())]);
        grid_frame(&context, grid, &mut editor, &mut ui_state, vec![wheel]);
        for _ in 0..60 {
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        }
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        assert_eq!(ui_state.graph.view().zoom, zoom, "the wheel over the tabs zoomed the graph");
        let names = tab_names(&shapes, strip);
        assert!(tab_name(&names, "Graph 6").is_some_and(in_full_sight), "the tabs did not scroll to Graph 6");
        assert!(!tab_name(&names, "Graph 1").is_some_and(in_full_sight));
        // The shown tab stays where the wheel left it.
        assert!(ui_state.graph.tab_strip.scroll > 0.0);
    }

    #[test]
    fn a_long_tab_name_is_cut_short_rather_than_pushing_the_others_out() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let tabs = six_tabs(&mut editor);
        let long = "Erosion and Hydrology Experiments Tab With Long Name";
        editor.document.rename_graph_tab(tabs[1].index, long);
        // The graph of a 1280 wide window with the Data View shut.
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(1000.0, 400.0));
        let strip = tab_strip(&context, grid);
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let names = tab_names(&shapes, strip);
        let shown = tab_name(&names, long).expect("the long name");
        assert!(shown.3, "the long name is not cut short");
        assert!(in_full_sight(shown));
        for tab in tabs.iter().filter(|tab| tab.index != tabs[1].index) {
            let name = tab_name(&names, &tab.name).unwrap_or_else(|| panic!("{} is not painted", tab.name));
            assert!(in_full_sight(name) && !name.3, "{} is not in full sight", tab.name);
        }
    }

    #[test]
    fn ticks_fit_before_the_end() {
        assert_eq!(ticks_that_fit(100.0, 400.0), FOOTER_TICKS);
        assert_eq!(ticks_that_fit(100.0, 101.0), 1);
        assert_eq!(ticks_that_fit(100.0, 100.5), 0);
        assert_eq!(ticks_that_fit(100.0, 50.0), 0);
        // Ticks at 100, 106, 112 and 118: the last ends at 119.
        assert_eq!(ticks_that_fit(100.0, 120.0), 4);
        assert_eq!(ticks_that_fit(100.0, 118.5), 3);
    }
}
