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
use crate::ui::graph_commands::GraphCommand;
use crate::ui::graph_popups::swatch_grid;
use crate::ui::prompt::{task_actions, TaskDialog};
use crate::ui::widgets::{icon_button, icon_menu, menu_caption, menu_item, menu_item_enabled, menu_separator, menu_style, submenu, toolbar_background_corners, toolbar_separator, vertical_toolbar, Tooltips};
use crate::ui::{data_view, graph_canvas, menus, toolbox, UiState};
use crate::user::ToolboxStyle;

/// Height of the tab strip (`GraphTabsGrid`) and of a tab (`GraphTabItem`).
const TABS_HEIGHT: f32 = 26.0;
const TAB_HEIGHT: f32 = 25.0;
/// `TabBorderThickness`: tab outlines, the line under the tabs and the
/// graph's border.
const TAB_BORDER: f32 = 1.5;
/// `GraphBorder`'s top margin below the tabs.
const GRAPH_TOP: f32 = 25.0;
/// `GraphToolbarBorder` (`Toolbar`): a `Toolbar.ButtonSize` button,
/// `Toolbar.Padding` and a border of 1.5 each side.
fn toolbar_width(sizes: &theme::Sizes) -> f32 {
    sizes.toolbar_button + 2.0 * (sizes.toolbar_padding[0] + 1.5)
}
/// Each area's margin in the grid.
const AREA_MARGIN: f32 = 2.0;
/// `gsVertical1`: 4 wide at the graph column's right edge.
const SPLITTER: f32 = 4.0;

pub fn show(ui: &mut Ui, graph_grid: Rect, editor: &mut Editor, ui_state: &mut UiState, commands: &mut Vec<Command>) {
    // The toolbar column (auto): margin 2, or 0, 2, 2, 2 against the open
    // Data View (`ToggleCollapse`).
    let data_shown = ui_state.data_view.expanded;
    let toolbar_left_margin = if data_shown { 0.0 } else { AREA_MARGIN };
    let toolbar_column_width = toolbar_left_margin + toolbar_width(theme::sizes(ui.ctx())) + AREA_MARGIN;
    let toolbar_column = Rect::from_min_max(Pos2::new(graph_grid.right() - toolbar_column_width, graph_grid.top()), graph_grid.max);
    let toolbar_rect = Rect::from_min_max(toolbar_column.min + Vec2::new(toolbar_left_margin, AREA_MARGIN), toolbar_column.max - Vec2::splat(AREA_MARGIN));

    // The star columns: the graph `5*`, the Data View `1.05*` (at least
    // 340) while open.
    let stars = (toolbar_column.left() - graph_grid.left()).max(0.0);
    let data_width = if data_shown { data_view_width(stars, ui_state.data_view.share) } else { 0.0 };

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
    // panel; none without tabs. The build strip is drawn over its foot. It
    // is placed before the graph lays out, and drawn over it.
    let menus = editor.user.options.toolbox_style() == ToolboxStyle::Menus;
    let toolbox_area = if tabs_hidden {
        grid
    } else if menus {
        Rect::from_min_max(grid.min + Vec2::new(0.0, 35.0), grid.max)
    } else {
        Rect::from_min_max(grid.min + Vec2::new(0.0, 16.0), grid.max - Vec2::new(0.0, 24.0))
    };
    ui_state.graph.toolbox_rect = toolbox::place(ui, toolbox_area, &editor.user.options);
    graph_canvas::show(ui, graph_rect, editor, &mut ui_state.graph, commands);
    toolbox::show(ui, toolbox_area, editor, &mut ui_state.graph, commands);
    if !tabs_hidden {
        tabs(ui, grid, editor, ui_state, selected_color, commands);
    }
    // `stackBookmarks` stays (margin 0, 0, 2, 0; 8 on the right without
    // tabs), over the tabs' end.
    let bookmarks_margin = if tabs_hidden { 8.0 } else { 2.0 };
    bookmark_buttons(ui, grid, bookmarks_margin, editor, commands);
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

/// The Data View's width in the star space: its share, at least 340
/// (`DataViewColumn.MinWidth`); the graph column has no least width. In
/// stars narrower than that it takes them all.
fn data_view_width(stars: f32, share: f32) -> f32 {
    (stars * share).max(data_view::MIN_WIDTH).min(stars)
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

/// The graph tabs (`GraphTabList`, 1 down): each at least 135 wide, its
/// colour bar 12 in and its name (monospace 11) 24 in. The chosen tab is
/// outlined in its colour and tinted with it; the others, and the rest of
/// the strip, stand on a line of the chosen tab's colour. Each tab's menu
/// offers Color, Rename and Delete; the strip's offers New Tab.
fn tabs(ui: &mut Ui, area: Rect, editor: &Editor, ui_state: &mut UiState, selected_color: egui::Color32, commands: &mut Vec<Command>) {
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
    let line_stroke = Stroke::new(TAB_BORDER, selected_color);
    let line_y = strip.top() + 1.0 + TAB_HEIGHT - TAB_BORDER / 2.0;
    let mut x = strip.left();
    for tab in &tabs {
        let swatch = theme::swatch_color(&tab.color);
        let selected = ui_state.graph.tab == tab.index;
        let galley = ui.painter().layout_no_wrap(tab.name.clone(), theme::monospace(11.0), theme::LABEL_EMPHASIS);
        let width = (galley.size().x + 48.0).max(135.0);
        let tab_rect = Rect::from_min_size(Pos2::new(x, strip.top() + 1.0), Vec2::new(width, TAB_HEIGHT));
        let response = ui.interact(tab_rect, ui.id().with(("graph-tab", tab.index)), Sense::click());
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
                let text_position = Pos2::new(tab_rect.left() + 24.0, tab_rect.center().y - galley.size().y / 2.0);
                ui.painter().galley(text_position, galley, theme::LABEL_EMPHASIS);
            }
        }
        // Tabs overlap by 1 (`Margin 0,0,-1,0`).
        x += width - 1.0;
    }
    // `ExtraBorder`: the line on to the end of the strip.
    ui.painter().line_segment([Pos2::new(x, line_y), Pos2::new(strip.right(), line_y)], line_stroke);

    confirm_tab_delete(ui, &tabs, ui_state, commands);
}

/// `stackBookmarks` (`UpdateBookmarkMenu`): a small bar for each bookmark
/// at the right of the tab strip, `right_margin` in, in its node's toolbox
/// colour, its name as the tooltip; a click goes to the node.
/// `HideBookmarkShortcuts` hides them.
fn bookmark_buttons(ui: &mut Ui, area: Rect, right_margin: f32, editor: &Editor, commands: &mut Vec<Command>) {
    if editor.user.options.hide_bookmark_shortcuts() {
        return;
    }
    let bookmarks = editor.document.bookmarks();
    let nodes: Vec<&nerthus_engine::NodeData> = bookmarks.iter().filter_map(|id| editor.document.node(*id)).collect();
    // `BookmarkButton`: padding 2 around a 6 by 12 bar, margin 2, 0.
    let button_size = Vec2::new(6.0 + 4.0, 12.0 + 4.0);
    let mut right = area.right() - right_margin;
    let center_y = area.top() + TABS_HEIGHT / 2.0;
    for node in nodes.iter().rev() {
        let rect = Rect::from_min_max(Pos2::new(right - 2.0 - button_size.x, center_y - button_size.y / 2.0), Pos2::new(right - 2.0, center_y + button_size.y / 2.0));
        right = rect.left() - 2.0;
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
    let heading = format!("Delete Group \"{name}\" ?");
    let modal = TaskDialog::new(ui.ctx(), egui::Id::new("confirm-delete-tab"), 350.0);
    let response = modal.show(ui.ctx(), |ui, task| {
        task.body(ui, &heading, false, |ui| {
            ui.add(egui::Label::new(format!("Are you sure you want to delete this Graph \"{name}\" ")).wrap());
        });
        task.buttons(ui, |ui| {
            match task_actions(ui, &["Yes", "No"]) {
                Some(0) => {
                    commands.push(Command::DeleteGraphTab(index));
                    ui_state.graph.confirm_delete_tab = None;
                }
                Some(_) => ui_state.graph.confirm_delete_tab = None,
                None => {}
            }
        });
    });
    if response.should_close() {
        ui_state.graph.confirm_delete_tab = None;
    }
}

/// The build strip (`Border`): 26 high, at least 128 wide, margin 1.75,
/// `Panel-Cladding`, corners 0, 4, 0, 4.
const FOOTER_HEIGHT: f32 = 26.0;
const FOOTER_MIN_WIDTH: f32 = 128.0;
const FOOTER_INSET: f32 = 1.75;
/// A `ProgressX` cell: a 2-wide line or a 2 by 2 dot with its margins of
/// 0.75.
const PROGRESS_CELL: f32 = 3.5;
const PROGRESS_SLICE: f32 = 2.0;
/// `Progress`: twenty lines (`DotsCount`) from 0 to 100, margin 6, 6, 0, 6.
const PROGRESS_LINES: usize = 20;
/// `NodeProgress`: a dot a node in four rows, margin 6, 6, 12, 6.
const NODE_PROGRESS_ROWS: usize = 4;

/// The strip's parts, left to right as its `DockPanel` lays them out:
/// `StateIcon` (margin 8, 0, 4, 0), `TotalTime`, the separator (1 wide,
/// margin 8, 0, 2, 0), `Progress` filling what `NodeProgress`, docked
/// right, leaves.
struct FooterLayout {
    rect: Rect,
    icon: Rect,
    clock: Pos2,
    separator_x: f32,
    lines_left: f32,
    dots_left: f32,
    dot_columns: usize,
}

fn footer_layout(area: Rect, clock_width: f32, nodes: usize) -> FooterLayout {
    let dot_columns = nodes.max(1).div_ceil(NODE_PROGRESS_ROWS);
    let lines_width = PROGRESS_LINES as f32 * PROGRESS_CELL;
    let dots_width = dot_columns as f32 * PROGRESS_CELL;
    let parts = 8.0 + 16.0 + 4.0 + clock_width + 8.0 + 1.0 + 2.0 + 6.0 + lines_width + 6.0 + dots_width + 12.0;
    let width = parts.max(FOOTER_MIN_WIDTH);
    let rect = Rect::from_min_size(Pos2::new(area.left() + FOOTER_INSET, area.bottom() - FOOTER_INSET - FOOTER_HEIGHT), Vec2::new(width, FOOTER_HEIGHT));
    let icon = Rect::from_min_size(Pos2::new(rect.left() + 8.0, rect.center().y - 8.0), Vec2::splat(16.0));
    let clock = Pos2::new(icon.right() + 4.0, rect.center().y);
    let separator_x = clock.x + clock_width + 8.0;
    let lines_left = separator_x + 1.0 + 2.0 + 6.0;
    // `NodeProgress` docks right; `Progress` fills the rest (wider than
    // its lines only when the strip is held to 128).
    let dots_left = rect.right() - 12.0 - dots_width;
    FooterLayout { rect, icon, clock, separator_x, lines_left, dots_left, dot_columns }
}

/// The build clock and progress at the bottom left of the graph `area`,
/// cut off at its edge (`StateIcon`, `TotalTime`, `Progress`,
/// `NodeProgress`).
fn footer(ui: &mut Ui, area: Rect, editor: &Editor) {
    let seconds = editor.build_seconds();
    let minutes = (seconds / 60.0).floor() as u32;
    let remainder = (seconds % 60.0).floor() as u32;
    let clock = format!("{minutes:02}:{remainder:02}");
    let galley = ui.painter().layout_no_wrap(clock, theme::monospace_semibold(9.0), theme::LABEL_DEFAULT);
    let nodes = editor.document.terrain().nodes.keys().copied().collect::<Vec<i32>>();
    let layout = footer_layout(area, galley.size().x, nodes.len());
    let visible = Rect::from_min_max(layout.rect.min, Pos2::new(layout.rect.right().min(area.right() - FOOTER_INSET), layout.rect.bottom()));
    let painter = ui.painter().with_clip_rect(visible.intersect(ui.clip_rect()));
    let corners = egui::CornerRadius { nw: 0, ne: 4, se: 0, sw: 4 };
    painter.rect_filled(layout.rect, corners, theme::PANEL_CLADDING);

    // `SetStateIcon`: the heartbeat, blue while building, the engine off
    // in red while suspended.
    let (icon, icon_color) = if editor.cancelling && editor.building {
        ("circle-dashed-x", theme::ACCENT_YELLOW)
    } else if !editor.engine_active {
        ("engine-off", theme::ACCENT_RED)
    } else if editor.building {
        ("heartbeat", theme::ACCENT_BLUE)
    } else {
        ("activity-heartbeat", theme::ICON_DEFAULT)
    };
    let mut icon_ui = ui.new_child(UiBuilder::new().max_rect(layout.icon));
    icon_ui.set_clip_rect(painter.clip_rect());
    if editor.building && !editor.cancelling && editor.engine_active {
        // Status-IconX's ShineBoard moves a bright opacity mask across
        // the heartbeat every half-second, leaving its outline at 20%.
        let time = ui.input(|state| state.time) as f32;
        let clip = icon_ui.clip_rect();
        for slice in 0..16 {
            let left = layout.icon.left() + slice as f32;
            let slice_rect = Rect::from_min_max(Pos2::new(left, layout.icon.top()), Pos2::new(left + 1.0, layout.icon.bottom()));
            icon_ui.set_clip_rect(slice_rect.intersect(clip));
            let opacity = build_icon_opacity(time, (slice as f32 + 0.5) / 16.0);
            icons::paint(&icon_ui, layout.icon, icon, icon_color.gamma_multiply(opacity));
        }
    } else {
        icons::paint(&icon_ui, layout.icon, icon, icon_color);
    }
    painter.galley(Pos2::new(layout.clock.x, layout.clock.y - galley.size().y / 2.0), galley, theme::LABEL_DEFAULT);

    let center = layout.rect.center().y;
    let stroke = Stroke::new(1.0, theme::MENU_SEPARATOR);
    let separator_x = layout.separator_x + 0.5;
    painter.line_segment([Pos2::new(separator_x, center - 4.0), Pos2::new(separator_x, center + 4.0)], stroke);

    // `Progress` (Lines): the building node's share of 100 lights its lines in
    // `Accent-Blue`.
    let progress = editor.building_node.map(|node| editor.status(node).progress).unwrap_or(0.0);
    let progress = if editor.building { progress.clamp(0.0, 1.0) } else { 0.0 };
    let lit = (PROGRESS_LINES as f32 * progress) as usize;
    let lines_top = layout.rect.top() + 6.0 + 0.75;
    let lines_bottom = layout.rect.bottom() - 6.0 - 0.75;
    for line in 0..PROGRESS_LINES {
        let left = layout.lines_left + line as f32 * PROGRESS_CELL + 0.75;
        let color = if line < lit { theme::ACCENT_BLUE } else { theme::PROGRESS_EMPTY };
        let rect = Rect::from_min_max(Pos2::new(left, lines_top), Pos2::new(left + PROGRESS_SLICE, lines_bottom));
        painter.rect_filled(rect, 1.0, color);
    }

    // `NodeProgress` (Dots): a dot a node, row by row; up to the count of
    // built nodes in `Accent-Purple` and, while building, the next in
    // `Accent-Primary`.
    let built = nodes.iter().filter(|id| editor.status(**id).state == nerthus_engine::graph::NodeState::Built).count();
    let dots = nodes.len().max(1);
    let dots_top = layout.rect.top() + 6.0;
    for dot in 0..dots {
        let row = dot / layout.dot_columns;
        let column = dot % layout.dot_columns;
        let min = Pos2::new(layout.dots_left + column as f32 * PROGRESS_CELL + 0.75, dots_top + row as f32 * PROGRESS_CELL + 0.75);
        let color = if editor.building && dot == built {
            theme::ACCENT_PRIMARY
        } else if dot <= built {
            theme::ACCENT_PURPLE
        } else {
            theme::PROGRESS_EMPTY
        };
        painter.rect_filled(Rect::from_min_size(min, Vec2::splat(PROGRESS_SLICE)), 1.0, color);
    }
    if editor.building {
        ui.ctx().request_repaint();
    }
}

/// `Status-IconX.ShineBoard`: the three gradient stops' offsets and opacity.
fn build_icon_opacity(time: f32, position: f32) -> f32 {
    let phase = time.rem_euclid(0.5);
    let outer_time = phase / 0.5;
    let inner_time = (phase / 0.45).min(1.0);
    let left = -0.65 + 1.9 * outer_time;
    let peak = -0.4 + 1.7 * inner_time;
    let right = -0.2 + 1.51 * inner_time;
    let strength = if position < peak {
        (position - left) / (peak - left)
    } else {
        (right - position) / (right - peak)
    };
    0.2 + 0.8 * strength.clamp(0.0, 1.0)
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
        // `btnExpandToggle`.
        let chevron = if data_shown { "chevron-right" } else { "chevron-left" };
        if icon_button(column, chevron, "Data View", None).clicked() {
            ui_state.data_view.toggle();
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
        // and Graph Commands, with separators (margin 2, 8) between.
        dock.bottom(column);
        let refresh = icon_button(column, "rotate-360", "Refresh", None);
        if refresh.clicked() {
            let command = if editor.selection.is_empty() { Command::RefreshAll } else { Command::Refresh };
            commands.push(command);
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

    #[test]
    fn delete_tab_confirmation_defaults_to_yes_and_tab_selects_no() {
        for activation in [egui::Key::Enter, egui::Key::Space] {
            for choose_no in [false, true] {
                let context = egui::Context::default();
                let mut ui_state = UiState::default();
                ui_state.graph.confirm_delete_tab = Some(1);
                let mut commands = Vec::new();
                let key = |key| egui::Event::Key {
                    key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE,
                };
                let mut frames = vec![Vec::new()];
                if choose_no {
                    frames.push(vec![key(egui::Key::Tab)]);
                }
                frames.push(vec![key(activation)]);
                for events in frames {
                    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
                    let input = egui::RawInput { screen_rect: Some(screen), events, ..Default::default() };
                    let output = context.run_ui(input, |ui| confirm_tab_delete(ui, &[], &mut ui_state, &mut commands));
                    output.drop_without_applying_deltas();
                }
                assert!(ui_state.graph.confirm_delete_tab.is_none());
                let expected = if choose_no { Vec::new() } else { vec![Command::DeleteGraphTab(1)] };
                assert_eq!(commands, expected);
            }
        }
    }

    /// The build strip painted at the bottom of a graph frame `width` wide,
    /// in a terrain of `nodes` nodes: the frame, the strip as laid out, and
    /// the shapes painted.
    fn painted_footer(width: f32, nodes: usize) -> (Rect, Rect, Vec<egui::epaint::ClippedShape>) {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut document = Document::new();
        for node in 0..nodes {
            document.add_node("Constant", [26000.0 + 300.0 * node as f32, 26000.0]);
        }
        let editor = Editor::new(document);
        let area = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(width, 400.0));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 960.0))), ..Default::default() };
        let mut strip = Rect::NOTHING;
        let mut output = context.run_ui(input, |ui| {
            let clock = ui.painter().layout_no_wrap("00:00".to_string(), theme::monospace_semibold(9.0), theme::LABEL_DEFAULT).size().x;
            strip = footer_layout(area, clock, nodes).rect;
            footer(ui, area, &editor);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        (area, strip, shapes)
    }

    /// The progress lines (2 wide) and dots (2 by 2) painted.
    fn progress_marks(shapes: &[egui::epaint::ClippedShape]) -> (Vec<Rect>, Vec<Rect>) {
        let slices = shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Rect(rect) if (rect.rect.width() - PROGRESS_SLICE).abs() < 0.01 => Some(rect.rect),
            _ => None,
        });
        slices.partition(|rect| rect.height() > PROGRESS_SLICE + 0.01)
    }

    #[test]
    fn the_build_strip_is_as_wide_as_its_parts() {
        for nodes in [0, 3, 20, 41] {
            let (area, strip, shapes) = painted_footer(900.0, nodes);
            assert_eq!(strip.left(), area.left() + FOOTER_INSET);
            assert_eq!(strip.bottom(), area.bottom() - FOOTER_INSET);
            assert!(strip.width() >= FOOTER_MIN_WIDTH);
            let (lines, dots) = progress_marks(&shapes);
            // Twenty lines a cell apart, then a dot a node in four rows.
            assert_eq!(lines.len(), PROGRESS_LINES, "{nodes} nodes");
            for pair in lines.windows(2) {
                assert!((pair[1].left() - pair[0].left() - PROGRESS_CELL).abs() < 0.01);
            }
            assert_eq!(dots.len(), nodes.max(1), "{nodes} nodes");
            let columns = nodes.max(1).div_ceil(NODE_PROGRESS_ROWS) as f32;
            let dots_right = dots.iter().map(|dot| dot.right()).fold(f32::MIN, f32::max);
            assert!((strip.right() - 12.0 - 0.75 - dots_right).abs() < 0.01, "{nodes} nodes: the dots end at {dots_right}");
            let dots_left = dots.iter().map(|dot| dot.left()).fold(f32::MAX, f32::min);
            assert!((dots_right - dots_left - (columns * PROGRESS_CELL - 1.5)).abs() < 0.01, "{nodes} nodes");
        }
    }

    #[test]
    fn the_footer_lines_follow_the_building_node_instead_of_finished_node_counts() {
        let context = egui::Context::default();
        theme::apply(&context);
        let mut document = Document::new();
        let node = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let mut editor = Editor::new(document);
        editor.building = true;
        editor.building_node = Some(node);
        editor.build_progress = 0.9;
        let status = crate::editor::NodeStatus { building: true, progress: 0.5, ..Default::default() };
        editor.statuses.insert(node, status);
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 400.0));
        let output = context.run_ui(egui::RawInput::default(), |ui| footer(ui, area, &editor));
        let lit_lines = output.shapes.iter().filter(|clipped| match &clipped.shape {
            Shape::Rect(shape) => shape.fill == theme::ACCENT_BLUE && shape.rect.height() > PROGRESS_SLICE,
            _ => false,
        }).count();
        output.drop_without_applying_deltas();
        assert_eq!(lit_lines, PROGRESS_LINES / 2);
    }

    #[test]
    fn a_narrow_graph_cuts_the_strip_off_at_its_edge() {
        for width in [150.0, 120.0, 80.0, 40.0] {
            let (area, strip, shapes) = painted_footer(width, 3);
            let wide = painted_footer(900.0, 3).1;
            assert_eq!(strip.width(), wide.width(), "{width}: the strip narrowed");
            let edge = area.right() - FOOTER_INSET;
            for clipped in &shapes {
                assert!(clipped.clip_rect.right() <= edge + 0.01, "{width}: {:?} painted to {}", clipped.shape, clipped.clip_rect.right());
            }
        }
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

    /// The toolbar column's width while the Data View is shut.
    fn shut_toolbar_column(context: &egui::Context) -> f32 {
        AREA_MARGIN + toolbar_width(theme::sizes(context)) + AREA_MARGIN
    }

    /// Where the Data View chevron, the toolbar's first button, sits.
    fn chevron_at(context: &egui::Context, grid: Rect) -> Pos2 {
        let sizes = theme::sizes(context);
        let toolbar_right = grid.right() - AREA_MARGIN;
        let x = toolbar_right - toolbar_width(sizes) / 2.0;
        let y = grid.top() + AREA_MARGIN + 1.5 + sizes.toolbar_padding[1] + sizes.toolbar_button / 2.0;
        Pos2::new(x, y)
    }


    #[test]
    fn the_data_view_keeps_its_least_width() {
        let share = data_view::DEFAULT_SHARE;
        // Its share, at least 340; the graph has the rest.
        assert_eq!(data_view_width(1000.0, share), data_view::MIN_WIDTH);
        assert_eq!(data_view_width(3000.0, share), 3000.0 * share);
        assert_eq!(data_view_width(800.0, 0.9), 720.0);
        assert_eq!(data_view_width(450.0, share), data_view::MIN_WIDTH);
        // Dragged all the way, it takes every star; narrower stars than
        // 340, all there are.
        assert_eq!(data_view_width(800.0, 1.0), 800.0);
        assert_eq!(data_view_width(300.0, share), 300.0);
    }

    #[test]
    fn the_graph_has_what_the_open_data_view_leaves() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        ui_state.data_view.expanded = true;
        for width in (250..=1400).step_by(10) {
            let grid = Rect::from_min_size(Pos2::new(4.0, 300.0), Vec2::new(width as f32, 400.0));
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
            let data = ui_state.graph.areas.data_view.expect("the Data View shows while open");
            let stars = grid.width() - (toolbar_width(theme::sizes(&context)) + AREA_MARGIN);
            let expected = data_view_width(stars, ui_state.data_view.share);
            assert!((data.width() + AREA_MARGIN - expected).abs() < 1.0, "{width}: the Data View is {} wide, not {expected}", data.width() + AREA_MARGIN);
            if stars > data_view::MIN_WIDTH {
                assert!(data.left() >= ui_state.graph.rect.left() - 0.5, "{width}");
            }
        }
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
    fn the_toolbox_menus_keep_inside_the_graph() {
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
                    if toolbox.height() > 0.0 {
                        assert!(toolbox.bottom() <= grid.bottom() - 10.0 + 0.5, "{case}: the toolbox reaches {} past its margin", toolbox.bottom());
                    }
                    let frame = graph_frame(grid, tabs_hidden);
                    let buttons = toolbox_buttons(&shapes);
                    for button in &buttons {
                        assert!(toolbox.expand(0.5).contains_rect(*button), "{case}: a button at {button:?} past the toolbox {toolbox:?}");
                        assert!(frame.contains_rect(*button), "{case}: a button at {button:?} outside the graph frame {frame:?}");
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
        // Below the cut-off column: no category's menu.
        let (context, mut editor, mut ui_state) = graph_grid_context();
        editor.user.options.set_toolbox_style(ToolboxStyle::Menus);
        editor.user.options.set_toolbox_mode(ToolboxMode::Single);
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 200.0));
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let toolbox = ui_state.graph.toolbox_rect;
        for y in [toolbox.bottom() + 4.0, grid.bottom() + 4.0] {
            click(&context, grid, &mut editor, &mut ui_state, Pos2::new(toolbox.center().x, y));
            assert!(!egui::Popup::is_any_open(&context), "a menu opened at {y}");
        }
    }

    #[test]
    fn a_node_dragged_out_of_a_toolbox_menu_lands_where_it_is_let_go() {
        for nested in [true, false] {
            let (context, mut editor, mut ui_state) = graph_grid_context();
            editor.user.options.set_toolbox_style(ToolboxStyle::Menus);
            editor.user.options.set_toolbox_mode(ToolboxMode::Expanded);
            editor.user.options.set_toolbox_nested(nested);
            let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(1200.0, 700.0));
            grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
            let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
            let primitive = toolbox_text(&shapes, "Primitive").expect("Primitive in sight");
            click(&context, grid, &mut editor, &mut ui_state, primitive.center());
            if nested {
                let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
                let basic = toolbox_text(&shapes, "BASIC").expect("Basic in Primitive's menu");
                for _ in 0..5 {
                    grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(basic.center())]);
                }
            }
            let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
            let constant = toolbox_text(&shapes, "Constant").expect("Constant in the menu").center();
            // Pressed on Constant and dragged out over the empty graph: the
            // menu closes and the node's name follows the pointer.
            let button = |pos, pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(constant)]);
            grid_frame(&context, grid, &mut editor, &mut ui_state, vec![button(constant, true)]);
            grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(constant + Vec2::new(12.0, 0.0))]);
            let graph = ui_state.graph.rect;
            let drop = graph.center();
            let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, vec![egui::Event::PointerMoved(drop)]);
            assert_eq!(ui_state.graph.toolbox_drag.as_deref(), Some("Constant"), "nested: {nested}");
            assert!(!egui::Popup::is_any_open(&context), "nested: {nested}: the menu stayed open");
            let preview = toolbox_text(&shapes, "Constant").expect("the dragged node's name");
            assert!(preview.left() > drop.x && preview.top() > drop.y, "nested: {nested}: the name at {preview:?}, the pointer at {drop:?}");
            // Let go there, it is added there.
            let (_, commands) = grid_frame(&context, grid, &mut editor, &mut ui_state, vec![button(drop, false)]);
            let at = ui_state.graph.view().to_canvas(graph.min, drop);
            let added = GraphCommand::ToolboxDrop { type_name: "Constant".to_string(), at: [at.x, at.y], onto: None };
            assert_eq!(commands, vec![Command::Graph(added)], "nested: {nested}");
            assert_eq!(ui_state.graph.toolbox_drag, None);
        }
    }

    #[test]
    fn a_short_toolbar_keeps_its_foot_and_cuts_the_bottom_group_off_above() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let grid = Rect::from_min_size(Pos2::new(10.0, 300.0), Vec2::new(900.0, 250.0));
        let chevron = chevron_at(&context, grid);
        // Above the toolbar, where the viewport toolbar is, nothing of the
        // graph's takes a click.
        for y in (240..300).step_by(6) {
            let commands = click(&context, grid, &mut editor, &mut ui_state, Pos2::new(chevron.x, y as f32));
            assert!(commands.is_empty(), "a click at {y} above the toolbar: {commands:?}");
        }
        // Graph Commands at the foot, the engine toggle over it with a
        // separator (17) between, then Mutate and Refresh, cut off where
        // the top group ends: the engine toggle shows, Mutate does not.
        let sizes = theme::sizes(&context);
        let foot = grid.bottom() - AREA_MARGIN - 1.5 - sizes.toolbar_padding[1];
        let engine = Pos2::new(chevron.x, foot - 1.5 * sizes.toolbar_button - 17.0);
        let commands = click(&context, grid, &mut editor, &mut ui_state, engine);
        assert!(commands.iter().any(|command| matches!(command, Command::ToggleEngine)), "{commands:?}");
        let mutate = Pos2::new(chevron.x, foot - 3.5 * sizes.toolbar_button - 2.0 * 17.0);
        let commands = click(&context, grid, &mut editor, &mut ui_state, mutate);
        assert!(!commands.iter().any(|command| matches!(command, Command::Graph(GraphCommand::MutateButton))), "{commands:?}");
    }

    #[test]
    fn toolbar_refresh_uses_all_nodes_when_the_selection_is_empty() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 800.0));
        let chevron = chevron_at(&context, grid);
        let sizes = theme::sizes(&context);
        let foot = grid.bottom() - AREA_MARGIN - 1.5 - sizes.toolbar_padding[1];
        let refresh = Pos2::new(chevron.x, foot - 3.5 * sizes.toolbar_button - 2.0 * 17.0);
        let commands = click(&context, grid, &mut editor, &mut ui_state, refresh);
        assert_eq!(commands, vec![Command::RefreshAll]);

        let node = editor.document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        editor.select(Some(node));
        let commands = click(&context, grid, &mut editor, &mut ui_state, refresh);
        assert_eq!(commands, vec![Command::Refresh]);
    }

    #[test]
    fn a_short_toolbar_keeps_the_data_view_chevron_clickable() {
        // Too short for both groups: the bottom one is cut off where the
        // top one ends.
        let (context, mut editor, mut ui_state) = graph_grid_context();
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(900.0, 150.0));
        let commands = click(&context, grid, &mut editor, &mut ui_state, chevron_at(&context, grid));
        assert!(!commands.iter().any(|command| matches!(command, Command::Refresh)));
        assert!(ui_state.data_view.expanded);
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        assert!(ui_state.graph.areas.data_view.is_some());
    }

    #[test]
    fn tabs_keep_their_width_and_are_cut_off_at_the_graphs_edge() {
        let (context, mut editor, mut ui_state) = graph_grid_context();
        for _ in 0..5 {
            editor.document.add_graph_tab();
        }
        let grid = Rect::from_min_size(Pos2::new(10.0, 10.0), Vec2::new(500.0, 400.0));
        grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let (shapes, _) = grid_frame(&context, grid, &mut editor, &mut ui_state, Vec::new());
        let area_right = grid.right() - shut_toolbar_column(&context) - AREA_MARGIN;
        let names: Vec<&egui::epaint::ClippedShape> = shapes.iter().filter(|clipped| matches!(&clipped.shape, Shape::Text(text) if text.galley.text().starts_with("Graph"))).collect();
        assert!(names.len() >= 2, "the tabs' names show");
        for clipped in names {
            let Shape::Text(text) = &clipped.shape else { unreachable!() };
            assert!(!text.galley.elided, "{} is cut short", text.galley.text());
            assert!(clipped.clip_rect.right() <= area_right + 0.5, "{} drawn past the graph's edge", text.galley.text());
        }
    }
}
