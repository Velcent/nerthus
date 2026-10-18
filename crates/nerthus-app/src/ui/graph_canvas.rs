//! The node graph (`GraphControl`): dark nodes
//! with the toolbox category's colour on the title, icon, ports and
//! outgoing connections, borders that show the build state, "circuit"
//! connections, groups and notes behind the nodes, and the mouse
//! handling (drag to move, the view scrolling at the graph's edge, or to
//! connect, marquee selection, Ctrl Shift drag to push elements aside,
//! wheel zoom, middle or right drag to pan, a connection dropped into the
//! node search or onto another output, files and toolbox entries dropped
//! onto the graph).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use egui::epaint::{PathShape, PathStroke};
use egui::{Color32, CursorIcon, Key, PointerButton, Pos2, Rect, Response, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder, Vec2};
use nerthus_engine::{Catalogue, NodeData, NodeState, PortData};
use serde_json::Value;

use crate::document::{grid_inputs, grid_outputs, node_tab, Document, ExportSettings, GroupData};
use crate::editor::Editor;
use crate::icons;
use crate::theme;
use crate::ui::commands::Command;
use crate::ui::create_connection::CreateConnection;
use crate::ui::graph_commands::{GraphCommand, NodeSource};
use crate::ui::graph_groups::{self, GroupDrag, GroupState};
use crate::ui::graph_popups::GraphPopup;
use crate::ui::graph_search::{SearchMode, SearchPopup};
use crate::ui::slider_popups;
use crate::ui::widgets::{checked_icon_item, menu_item, menu_item_enabled, menu_separator, menu_style, submenu, Tooltips};

/// `NodeElement` template sizes (Standard, Compact, Small).
const STANDARD_WIDTH: f32 = 230.0;
const STANDARD_MIN_HEIGHT: f32 = 80.0;
const COMPACT_WIDTH: f32 = 170.0;
const COMPACT_HEIGHT: f32 = 64.0;
const SMALL_SIZE: f32 = 64.0;
/// Port rows (`RowDefinition` 16) and the port grids' margins: a
/// standard node's 40 from the top, 20 from the foot and 8 from the side;
/// a compact or small node's 14 from the top, centred in what that
/// leaves, and 5 from the side.
const PORT_ROW: f32 = 16.0;
const PORTS_TOP: f32 = 40.0;
const PORTS_BOTTOM: f32 = 20.0;
const PORTS_SIDE: f32 = 8.0;
const COMPACT_PORTS_TOP: f32 = 14.0;
const COMPACT_PORTS_SIDE: f32 = 5.0;
/// `PortElement` and its dot (`ControlBorder`).
const PORT_WIDTH: f32 = 20.0;
const PORT_HEIGHT: f32 = 12.0;
const PORT_DOT: f32 = 8.0;
/// `PART_PortHitGrid`: 20 high, reaching 4 past the element's right; on a
/// small node's ports 14 wide.
const PORT_HIT_HEIGHT: f32 = 20.0;
const PORT_HIT_REACH: f32 = 4.0;
const SMALL_PORT_HIT_WIDTH: f32 = 14.0;
/// `ConnectionElement.PortGap`.
const PORT_GAP: f32 = 10.0;
/// The circuit's horizontal run out of and into ports.
const CIRCUIT_RUN: f32 = 24.0;
/// Below this zoom nodes draw simplified (`IsZoomedOut`).
pub(crate) const SIMPLIFIED_ZOOM: f32 = 0.4;
/// A simplified node's port: a block around its dot.
const SIMPLIFIED_PORT: Vec2 = Vec2::new(24.0, 16.0);
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 2.0;
/// The room Extents and Selection leave round the elements' corners,
/// then the room `FitRect` adds (`FitToScreenExtentMargin`).
const EXTENTS_MARGIN: f32 = 200.0;
const FIT_MARGIN: f32 = 30.0;
/// How far from a port a dragged connection still snaps to it.
const SNAP_DISTANCE: f32 = 18.0;
/// `SnapToOtherShapes`: how near (canvas units) an edge or a centre snaps.
const SNAP_ITEMS_DISTANCE: f32 = 36.0;
/// `Line-SnapLine-Stroke`.
const SNAP_LINE: Color32 = Color32::from_rgb(0x60, 0x53, 0x8f);
/// `HandleAutoPanning`: how near a side of the graph (screen) a moved
/// element's pointer steps the view that way (`AutoPanEdgeDistance`),
/// and the step's speed (`AutoPanSpeed`; `AutoPanningTickRate` is 1).
const AUTO_PAN_EDGE: f32 = 32.0;
const AUTO_PAN_SPEED: f32 = 0.15;
/// How far the canvas runs (`CanvasExtents`).
const CANVAS_EXTENTS: f32 = 96000.0;
/// The rename popup (F2): how far in from the element's top-left it
/// opens (in the element's own units), its least width and its text's
/// size (at 100%).
const RENAME_OFFSET: f32 = 10.0;
const RENAME_MIN_WIDTH: f32 = 96.0;
const RENAME_FONT_SIZE: f32 = 12.0;
/// `CreateCollapsedPortElement`: the port standing in for a compact or
/// small node's ports past the first when it has more than two.
pub const COLLAPSED_PORT: &str = "CollapsedPort";

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoomRequest {
    Extents,
    Reset,
    Selection,
    /// `PanToElementLocation`: a node centred, the zoom kept.
    Center(i32),
    /// The same for a note.
    CenterNote(i32),
    /// The same for a group.
    CenterGroup(i32),
}

/// What the rename popup (F2) is open on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Renamed {
    Node(i32),
    Group(i32),
}

/// A graph tab's view: the canvas point at the top-left corner
/// (`ViewportLocation`) and the scale (`ZoomFactor`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphView {
    pub location: Vec2,
    pub zoom: f32,
}

/// `GraphTab`'s defaults.
impl Default for GraphView {
    fn default() -> Self {
        Self {
            location: Vec2::new(26000.0, 26000.0),
            zoom: 1.0,
        }
    }
}

impl GraphView {
    pub(crate) fn to_screen(self, origin: Pos2, point: Pos2) -> Pos2 {
        origin + (point.to_vec2() - self.location) * self.zoom
    }

    pub(crate) fn to_canvas(self, origin: Pos2, point: Pos2) -> Pos2 {
        let offset = (point - origin) / self.zoom;
        (self.location + offset).to_pos2()
    }

    pub(crate) fn rect_to_screen(self, origin: Pos2, rect: Rect) -> Rect {
        Rect::from_min_max(self.to_screen(origin, rect.min), self.to_screen(origin, rect.max))
    }
}

/// Where a drag began: the point the button went down. By the frame egui
/// reports the drag, the pointer is already past the click distance (and
/// further on a slow frame), so its position there would lose that much.
pub(crate) fn drag_origin(ui: &Ui, response: &Response) -> Option<Pos2> {
    ui.input(|input| input.pointer.press_origin()).or_else(|| response.interact_pointer_pos())
}

/// Where the editor's areas were this frame, for the Lazy Menu to move
/// the pointer into (`SetCursorPosition`).
#[derive(Clone, Copy, Debug)]
pub struct GraphAreas {
    /// The docked 2D view, while it shows in the shell.
    pub viewport_2d: Option<Rect>,
    pub graph: Rect,
    pub properties: Rect,
    pub data_view: Option<Rect>,
}

impl Default for GraphAreas {
    fn default() -> Self {
        Self { viewport_2d: None, graph: Rect::NOTHING, properties: Rect::NOTHING, data_view: None }
    }
}

/// What a Lazy Menu entry moves the pointer to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LazyTarget {
    Viewport2D,
    Graph,
    Properties,
    Modifiers,
    DataView,
}

/// An interaction in progress.
#[derive(Clone, Debug)]
enum Drag {
    /// Moving the selection: where each node, the note and each group
    /// started and where the pointer started. `primary` is the node
    /// grabbed, which Snap to Items aligns.
    Elements {
        pointer_start: Pos2,
        primary: i32,
        nodes: Vec<(i32, [f32; 2])>,
        note: Option<(i32, [f32; 2])>,
        groups: Vec<(i32, [f32; 2])>,
    },
    /// Selecting with a rectangle (screen coordinates).
    Marquee { start: Pos2, additive: bool },
    /// A connection pulled out of an output, or backwards out of an input.
    /// A `sticky` one (a port double-clicked, `IsClickClick`) ends at the
    /// next click instead of the release. One pulled out of a compact
    /// node's collapsed ports (`collapsed`) starts at the first of them.
    FromOutput { node: i32, port: String, sticky: bool, collapsed: bool },
    FromInput { node: i32, port: String, sticky: bool, collapsed: bool },
    /// Ctrl Shift on empty graph (`DragArrange`): where the push began
    /// (canvas units), and what lay to the right of it then.
    Push { press: Pos2, elements: PushedElements },
}

/// What a push moves, and where each started (canvas units). A group's
/// nodes and notes move with the group rather than on their own.
#[derive(Clone, Debug, Default, PartialEq)]
struct PushedElements {
    nodes: Vec<(i32, [f32; 2])>,
    notes: Vec<(i32, [f32; 2])>,
    groups: Vec<(i32, [f32; 2])>,
}

impl PushedElements {
    fn is_empty(&self) -> bool {
        self.nodes.is_empty() && self.notes.is_empty() && self.groups.is_empty()
    }
}

/// The graph's interaction state between frames.
pub struct GraphState {
    /// The shown tab's `Index`.
    pub tab: usize,
    /// Each tab's view, by `Index`.
    pub views: HashMap<usize, GraphView>,
    drag: Option<Drag>,
    /// Clicked connections (by the input each feeds): Delete, Insert node,
    /// Convert to Portal.
    pub selected_connections: Vec<(i32, String)>,
    /// Clicked groups.
    pub selected_groups: Vec<i32>,
    pub show_grid: bool,
    pub snap_to_grid: bool,
    /// `GridUnit` (`Options.GridSize`): what nodes snap to.
    pub grid_unit: f32,
    /// The node or group the rename popup (F2) is open on, and the name
    /// it opens with until it has shown;
    /// [`GraphState::start_renaming`] and
    /// [`GraphState::start_renaming_group`] open it.
    renaming: Option<Renamed>,
    rename_start: Option<String>,
    pub zoom_request: Option<ZoomRequest>,
    /// Canvas point of the last right-click, where added nodes go.
    pub context_point: Option<Pos2>,
    middle_panning: bool,
    /// The canvas widget's screen rectangle last frame.
    pub rect: Rect,
    /// Whether the views came from the document yet.
    views_loaded: bool,
    pub selected_note: Option<i32>,
    pub note_drag: Option<crate::ui::graph_notes::NoteDrag>,
    /// The note the Note Editor shows and its text so far.
    pub editing_note: Option<(i32, String)>,
    /// The tab being renamed and its name so far.
    pub renaming_tab: Option<(usize, String)>,
    /// The tab whose deletion awaits confirmation.
    pub confirm_delete_tab: Option<usize>,
    /// Nodes the Data View's filter leaves out, drawn at half strength.
    pub dimmed: HashSet<i32>,
    /// The bookmark whose menu entry reads "Remove?" (right-clicked).
    pub bookmark_removing: Option<i32>,
    /// A menu or the search open over the graph.
    pub popup: Option<GraphPopup>,
    /// The Create Connection window, while open.
    pub create_connection: Option<CreateConnection>,
    /// Where the pointer is, while it is over the graph (screen).
    pub pointer: Option<Pos2>,
    /// Shift held this frame (Ungroup then takes the group's nodes too).
    pub shift_held: bool,
    /// `Options.SnapToPorts`: a dragged connection takes a port near the
    /// pointer, not only one under it.
    snap_to_ports: bool,
    /// A group being moved or resized.
    pub group_drag: Option<GroupDrag>,
    /// What each group shows while an element is dragged over or out of
    /// it (`GroupState`).
    pub group_states: HashMap<i32, GroupState>,
    /// Snap to Items' lines (canvas units) while a node is dragged.
    snap_lines: Vec<[Pos2; 2]>,
    /// A toolbox entry being dragged onto the graph (its node type), and
    /// the toolbox's own state.
    pub toolbox_drag: Option<String>,
    pub toolbox: crate::ui::toolbox::ToolboxState,
    /// Where the toolbox goes this frame, placed before the graph lays
    /// out: the wheel over it is the toolbox's, not the graph's, and
    /// Extents and Selection leave room for it.
    pub toolbox_rect: Rect,
    /// The editor's areas this frame, and where the Lazy Menu asked the
    /// pointer to go.
    pub areas: GraphAreas,
    pub lazy_cursor: Option<LazyTarget>,
    /// Lazy Menu › Toggle Skybox, for the viewport's settings.
    pub toggle_skybox: bool,
    /// Nodes shining (`NodeElement.Shine`) and when each started (egui's
    /// clock).
    pub shines: HashMap<i32, f64>,
    /// The document's view generation the views were last taken at.
    view_generation: u64,
}

impl Default for GraphState {
    fn default() -> Self {
        Self {
            tab: 0,
            views: HashMap::new(),
            drag: None,
            selected_connections: Vec::new(),
            selected_groups: Vec::new(),
            show_grid: false,
            snap_to_grid: false,
            grid_unit: 25.0,
            renaming: None,
            rename_start: None,
            zoom_request: None,
            context_point: None,
            middle_panning: false,
            rect: Rect::from_min_size(Pos2::ZERO, Vec2::splat(400.0)),
            views_loaded: false,
            selected_note: None,
            note_drag: None,
            editing_note: None,
            renaming_tab: None,
            confirm_delete_tab: None,
            dimmed: HashSet::new(),
            bookmark_removing: None,
            popup: None,
            create_connection: None,
            pointer: None,
            shift_held: false,
            snap_to_ports: true,
            group_drag: None,
            group_states: HashMap::new(),
            snap_lines: Vec::new(),
            toolbox_drag: None,
            toolbox: crate::ui::toolbox::ToolboxState::default(),
            toolbox_rect: Rect::NOTHING,
            areas: GraphAreas::default(),
            lazy_cursor: None,
            toggle_skybox: false,
            shines: HashMap::new(),
            view_generation: 0,
        }
    }
}

impl GraphState {
    pub fn view(&self) -> GraphView {
        self.views.get(&self.tab).copied().unwrap_or_default()
    }

    fn view_mut(&mut self) -> &mut GraphView {
        self.views.entry(self.tab).or_default()
    }

    /// Forgets views so the next frame reads them from the document.
    pub fn reset(&mut self) {
        self.views_loaded = false;
        self.tab = 0;
        self.drag = None;
        self.selected_connections.clear();
        self.selected_groups.clear();
        self.stop_renaming();
        self.selected_note = None;
        self.note_drag = None;
        self.editing_note = None;
        self.popup = None;
        self.create_connection = None;
        self.group_drag = None;
        self.group_states.clear();
        self.snap_lines.clear();
        self.context_point = None;
        self.middle_panning = false;
        self.toolbox_drag = None;
        self.renaming_tab = None;
        self.zoom_request = None;
    }

    /// Switches tabs with no selection or unfinished gesture from the old tab.
    pub fn switch_tab(&mut self, editor: &mut Editor, tab: usize) {
        if self.tab == tab {
            return;
        }
        self.tab = tab;
        self.clear_element_selection();
        editor.select(None);
        self.stop_renaming();
        self.renaming_tab = None;
        self.drag = None;
        self.note_drag = None;
        self.group_drag = None;
        self.snap_lines.clear();
        self.group_states.clear();
        self.context_point = None;
        self.middle_panning = false;
        self.popup = None;
        self.create_connection = None;
        self.zoom_request = None;
    }

    /// A newly created tab must not inherit a deleted tab's cached view.
    pub fn reset_tab_view(&mut self, tab: usize) {
        self.views.remove(&tab);
    }

    /// Undo and redo can remove the shown tab. Choose its nearest predecessor.
    pub fn reconcile_tabs(&mut self, editor: &mut Editor) {
        let tabs = editor.document.graph_tabs();
        self.views.retain(|index, _| tabs.iter().any(|tab| tab.index == *index));
        if !tabs.iter().any(|tab| tab.index == self.tab) {
            let previous = tabs.iter().filter(|tab| tab.index < self.tab).max_by_key(|tab| tab.index);
            let fallback = previous.or_else(|| tabs.first());
            if let Some(tab) = fallback {
                self.switch_tab(editor, tab.index);
            }
        }
        editor.selection.retain(|id| editor.document.node(*id).is_some_and(|node| node_tab(node) == self.tab));
    }

    /// Opens the rename popup (F2, `RenameNode`) on `node`, holding the
    /// node's own name: never text an earlier rename left, on this node
    /// or another.
    pub fn start_renaming(&mut self, editor: &Editor, node: i32) {
        let Some(data) = editor.document.node(node) else {
            return;
        };
        self.renaming = Some(Renamed::Node(node));
        self.rename_start = Some(data.name.clone());
    }

    /// Opens the rename popup on group `id` (`RenameGroup`), holding
    /// `name`.
    pub fn start_renaming_group(&mut self, id: i32, name: String) {
        self.renaming = Some(Renamed::Group(id));
        self.rename_start = Some(name);
    }

    /// What the rename popup (F2) is open on.
    pub(crate) fn renaming(&self) -> Option<Renamed> {
        self.renaming
    }

    /// Closes the rename popup (F2) without taking its text.
    fn stop_renaming(&mut self) {
        self.renaming = None;
        self.rename_start = None;
    }

    /// The connections, groups and note the graph has chosen go (the
    /// nodes are the editor's selection).
    pub fn clear_element_selection(&mut self) {
        self.selected_connections.clear();
        self.selected_groups.clear();
        self.selected_note = None;
    }

    /// Where actions outside the canvas menu add content: the current view.
    pub fn drop_position(&self) -> [f32; 2] {
        let view = self.view();
        let center = view.location + self.rect.size() / (2.0 * view.zoom);
        [center.x - STANDARD_WIDTH / 2.0, center.y - STANDARD_MIN_HEIGHT / 2.0]
    }

    /// Only a canvas menu choice uses its opening point.
    pub fn context_position(&self) -> [f32; 2] {
        self.context_point.map(|point| [point.x, point.y]).unwrap_or_else(|| self.drop_position())
    }

    /// An undo or redo that reopened the project brings its tabs' views
    /// back with it (`GraphTabSelectionChanged(IsLoading: true)` after the
    /// terrain is read again).
    fn follow_restored_views(&mut self, editor: &Editor) {
        let generation = editor.document.view_generation();
        if self.view_generation == generation {
            return;
        }
        self.view_generation = generation;
        for tab in editor.document.graph_tabs() {
            let Some(location) = tab.location else {
                continue;
            };
            let view = GraphView {
                location: Vec2::new(location[0], location[1]),
                zoom: tab.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            };
            self.views.insert(tab.index, view);
        }
    }

    fn load_views(&mut self, editor: &Editor) {
        if self.views_loaded {
            return;
        }
        self.views_loaded = true;
        let tabs = editor.document.graph_tabs();
        self.views.clear();
        for tab in &tabs {
            let location = tab.location.unwrap_or([26000.0, 26000.0]);
            let view = GraphView {
                location: Vec2::new(location[0], location[1]),
                zoom: tab.zoom.clamp(MIN_ZOOM, MAX_ZOOM),
            };
            self.views.insert(tab.index, view);
        }
        if !tabs.iter().any(|tab| tab.index == self.tab) {
            self.tab = editor.document.opening_tab();
        }
        if tabs.iter().find(|tab| tab.index == self.tab).is_some_and(|tab| tab.location.is_none()) {
            self.zoom_request = Some(ZoomRequest::Extents);
        }
    }
}

/// A port as laid out on its node (canvas units).
#[derive(Clone, Debug)]
pub(crate) struct PortLayout {
    pub name: String,
    label: String,
    pub is_input: bool,
    connected: bool,
    /// Where connections attach (the dot's outer edge).
    pub anchor: Pos2,
    /// The `PortElement` (20 by 12).
    element: Rect,
    dot: Rect,
    /// Where it takes the pointer ([`port_hit`]).
    hit: [Rect; 2],
    portal: Option<Portal>,
    /// `PortElement.FriendlyName`: what a portal pill reads.
    portal_name: String,
    /// A port of a compact node's collapsed side past the first: placed in
    /// the second row, where its connections attach, but not shown.
    hidden: bool,
    /// The collapsed side's stand-in (`PortElementType.Collapsed`).
    collapsed: bool,
}

/// `PortalState`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Portal {
    Publishing,
    Consuming,
}

fn portal_state(port: &PortData) -> Option<Portal> {
    match port.extra.get("PortalState").and_then(Value::as_str) {
        Some("Publishing") => Some(Portal::Publishing),
        Some("Consuming") => Some(Portal::Consuming),
        _ => None,
    }
}

/// A node as laid out on the canvas.
#[derive(Clone, Debug)]
pub(crate) struct NodeLayout {
    pub id: i32,
    pub rect: Rect,
    size: NodeSize,
    color: Color32,
    icon: String,
    inputs: Vec<PortLayout>,
    outputs: Vec<PortLayout>,
    mask: Option<PortLayout>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NodeSize {
    Standard,
    Compact,
    Small,
}

/// The node's `NodeSize`; a node without one draws at the user's
/// `DefaultNodeSize` (`DetermineScale`).
fn node_size(node: &NodeData, default_size: &str) -> NodeSize {
    let saved = node.base.get("NodeSize").and_then(Value::as_str).unwrap_or("Default");
    let size = if saved == "Default" { default_size } else { saved };
    match size {
        "Compact" => NodeSize::Compact,
        "Small" => NodeSize::Small,
        _ => NodeSize::Standard,
    }
}

/// The colour of a node's toolbox category.
pub fn node_color(node: &NodeData) -> Color32 {
    let spec = Catalogue::get().node(&node.type_name);
    let category = spec.and_then(|spec| spec.toolbox.as_deref()).unwrap_or("Utility");
    theme::toolbox_color(category)
}

/// `IconAssigner`: `x-<name>` when there is one, else the family's icon
/// (`Plugin`: `family-<family>`), else `square-rounded`.
pub fn node_icon(type_name: &str) -> String {
    let spec = Catalogue::get().node(type_name);
    let name = spec.map(|spec| spec.name.to_lowercase()).unwrap_or_else(|| type_name.to_lowercase());
    let icon = format!("x-{name}");
    if icons::exists(&icon) {
        return icon;
    }
    let family = spec.and_then(|spec| spec.family.as_deref()).map(str::to_lowercase);
    if let Some(family) = family {
        let family_icon = format!("family-{family}");
        if icons::exists(&family_icon) {
            return family_icon;
        }
    }
    "square-rounded".to_string()
}

fn port_label(port: &PortData) -> String {
    let display = port.extra.get("DisplayName").and_then(Value::as_str);
    display.map(str::to_string).unwrap_or_else(|| nerthus_engine::humanize::humanize_title(&port.name))
}

/// `PortElement` uses its display name even when the node's size hides
/// the visible label. Collapsed sides stand in for "All Ports".
fn port_tooltip(editor: &Editor, node: i32, port: &PortLayout) -> String {
    if port.collapsed {
        return "All Ports".to_string();
    }
    let required = editor.document.node(node).and_then(|node| node.port(&port.name)).is_some_and(PortData::is_required);
    if required { format!("{} (Required)", port.label) } else { port.label.clone() }
}

/// The port rows a side takes: a compact or small node shows two at most
/// (the first port and the collapsed rest).
fn side_rows(count: usize, compact: bool) -> usize {
    if compact { count.min(2) } else { count }
}

pub(crate) fn layout_node(node: &NodeData, default_size: &str) -> NodeLayout {
    let size = node_size(node, default_size);
    let compact = size != NodeSize::Standard;
    let inputs = grid_inputs(node);
    let outputs = grid_outputs(node);
    let mask_port = node.ports.iter().find(|port| port.is_input() && port.name == "Mask");
    let input_rows = side_rows(inputs.len(), compact);
    let output_rows = side_rows(outputs.len(), compact);
    let rows = input_rows.max(output_rows) as f32;
    let compact_ports_top = |height: f32| COMPACT_PORTS_TOP + (height - COMPACT_PORTS_TOP - rows * PORT_ROW) / 2.0;
    let (width, height, ports_top) = match size {
        NodeSize::Standard => (STANDARD_WIDTH, (PORTS_TOP + rows * PORT_ROW + PORTS_BOTTOM).max(STANDARD_MIN_HEIGHT), PORTS_TOP),
        NodeSize::Compact => (COMPACT_WIDTH, COMPACT_HEIGHT, compact_ports_top(COMPACT_HEIGHT)),
        NodeSize::Small => (SMALL_SIZE, SMALL_SIZE, compact_ports_top(SMALL_SIZE)),
    };
    let ports_side = if compact { COMPACT_PORTS_SIDE } else { PORTS_SIDE };
    let origin = Pos2::new(node.position[0], node.position[1]);
    let rect = Rect::from_min_size(origin, Vec2::new(width, height));

    let place = |name: &str, row: usize, is_input: bool| {
        let row_top = rect.top() + ports_top + row as f32 * PORT_ROW;
        let element_top = row_top + (PORT_ROW - PORT_HEIGHT) / 2.0;
        let element_left = if is_input { rect.left() + ports_side } else { rect.right() - ports_side - PORT_WIDTH };
        let element = Rect::from_min_size(Pos2::new(element_left, element_top), Vec2::new(PORT_WIDTH, PORT_HEIGHT));
        let dot_left = if is_input { element.left() } else { element.right() - PORT_DOT };
        let dot = Rect::from_min_size(Pos2::new(dot_left, element.center().y - PORT_DOT / 2.0), Vec2::splat(PORT_DOT));
        // RefreshPoints: the element's centre, `PortGap` outwards.
        let anchor_x = if is_input { element.center().x - PORT_GAP } else { element.center().x + PORT_GAP };
        let anchor = Pos2::new(anchor_x, element.center().y);
        let hit = port_hit(element, size == NodeSize::Small, is_input);
        PortLayout {
            name: name.to_string(),
            label: String::new(),
            is_input,
            connected: false,
            anchor,
            element,
            dot,
            hit,
            portal: None,
            portal_name: String::new(),
            hidden: false,
            collapsed: false,
        }
    };
    // `ConfigurePorts`: a compact or small node puts its first port in the
    // first row and the rest in the second; past two, the second row shows
    // the collapsed stand-in only.
    let side = |ports: &[&PortData], is_input: bool| {
        let collapsing = compact && ports.len() > 2;
        let mut layouts = Vec::new();
        for (index, port) in ports.iter().enumerate() {
            let row = if compact { index.min(1) } else { index };
            let mut layout = place(&port.name, row, is_input);
            layout.label = port_label(port);
            layout.connected = port.record.is_some();
            layout.portal = portal_state(port);
            layout.hidden = collapsing && index > 0;
            layouts.push(layout);
        }
        if collapsing {
            // Added after the ports are tagged: never "small".
            let mut stand_in = place(COLLAPSED_PORT, 1, is_input);
            stand_in.collapsed = true;
            stand_in.hit = port_hit(stand_in.element, false, is_input);
            layouts.push(stand_in);
        }
        layouts
    };
    let input_layouts = side(&inputs, true);
    let output_layouts = side(&outputs, false);
    let mask = mask_port.map(|port| {
        let element = Rect::from_min_size(Pos2::new(rect.left() + 6.0, rect.bottom() + 6.0), Vec2::new(PORT_WIDTH, PORT_HEIGHT));
        let dot = Rect::from_min_size(Pos2::new(element.left(), element.center().y - PORT_DOT / 2.0), Vec2::splat(PORT_DOT));
        let anchor = Pos2::new(element.center().x - PORT_GAP, element.center().y);
        PortLayout {
            name: port.name.clone(),
            label: port_label(port),
            is_input: true,
            connected: port.record.is_some(),
            anchor,
            element,
            dot,
            hit: port_hit(element, false, true),
            portal: portal_state(port),
            portal_name: String::new(),
            hidden: false,
            collapsed: false,
        }
    });

    NodeLayout {
        id: node.id,
        rect,
        size,
        color: node_color(node),
        icon: node_icon(&node.type_name),
        inputs: input_layouts,
        outputs: output_layouts,
        mask,
    }
}

/// A node's width and height on the canvas (`ActualWidth`,
/// `ActualHeight`), which placements and the layout read.
pub fn node_extent(node: &NodeData, default_size: &str) -> [f32; 2] {
    let layout = layout_node(node, default_size);
    [layout.rect.width(), layout.rect.height()]
}

impl NodeLayout {
    /// A port by name, hidden ones included (connections attach to them).
    pub(crate) fn port(&self, name: &str, is_input: bool) -> Option<&PortLayout> {
        if is_input {
            if let Some(mask) = self.mask.as_ref().filter(|mask| mask.name == name) {
                return Some(mask);
            }
            return self.inputs.iter().find(|port| port.name == name);
        }
        self.outputs.iter().find(|port| port.name == name)
    }

    /// The ports shown.
    fn ports(&self) -> impl Iterator<Item = &PortLayout> {
        let all = self.inputs.iter().chain(self.outputs.iter()).chain(self.mask.iter());
        all.filter(|port| !port.hidden)
    }
}

impl PortLayout {
    /// Whether the port takes the pointer at `point` (canvas).
    fn takes(&self, point: Pos2) -> bool {
        self.hit.iter().any(|area| area.contains(point))
    }
}

/// Where a port whose element is `element` takes the pointer: the element
/// (`MainGrid`, its background transparent) and `PART_PortHitGrid`, 20
/// high about the element's middle from its left to 4 past its right; on
/// a small node's ports (`Tag` "small") only the 14 of that on the dot's
/// side.
fn port_hit(element: Rect, small: bool, is_input: bool) -> [Rect; 2] {
    let top = element.center().y - PORT_HIT_HEIGHT / 2.0;
    let bottom = top + PORT_HIT_HEIGHT;
    let mut left = element.left();
    let mut right = element.right() + PORT_HIT_REACH;
    if small && is_input {
        right = left + SMALL_PORT_HIT_WIDTH;
    } else if small {
        left = right - SMALL_PORT_HIT_WIDTH;
    }
    let grid = Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom));
    [element, grid]
}

/// `PortElement.SetFriendlyName`: a publishing portal reads its node's
/// name, a consuming one its source's (`Node | Port` when the source port
/// is not the primary output).
fn name_portals(layouts: &mut [NodeLayout], editor: &Editor) {
    let nodes = &editor.document.terrain().nodes;
    for layout in layouts.iter_mut() {
        let Some(node) = nodes.get(&layout.id) else { continue };
        let ports = layout.inputs.iter_mut().chain(layout.outputs.iter_mut()).chain(layout.mask.iter_mut());
        for port in ports {
            match port.portal {
                Some(Portal::Publishing) => port.portal_name = node.name.clone(),
                Some(Portal::Consuming) => {
                    let record = node.port(&port.name).and_then(|data| data.record.as_ref());
                    let source = record.and_then(|record| nodes.get(&record.from).map(|source| (source, record.from_port.as_str())));
                    let Some((source, source_port)) = source else { continue };
                    let primary_out = source.port(source_port).is_some_and(|data| data.is_primary() && data.is_output());
                    port.portal_name = if primary_out { source.name.clone() } else { format!("{} | {source_port}", source.name) };
                }
                None => {}
            }
        }
    }
}

/// A connection as drawn: from an output to the input it feeds.
struct ConnectionLayout {
    to: i32,
    to_port: String,
    points: Vec<Pos2>,
    color: Color32,
}

/// The circuit path (`ConnectionType.Circuit`): out along the port, across,
/// and in; close nodes get the short "proximity" kink.
fn circuit_points(start: Pos2, end: Pos2, target_standard: bool) -> Vec<Pos2> {
    let close = target_standard && end.x < start.x + 60.0 && (end.y - start.y).abs() < 60.0;
    if close {
        return vec![start, Pos2::new(start.x + 8.0, start.y), Pos2::new(end.x - 8.0, end.y)];
    }
    vec![start, Pos2::new(start.x + CIRCUIT_RUN, start.y), Pos2::new(end.x - CIRCUIT_RUN, end.y), end]
}

fn distance_to_segment(point: Pos2, start: Pos2, end: Pos2) -> f32 {
    let segment = end - start;
    let length_squared = segment.length_sq();
    if length_squared <= f32::EPSILON {
        return point.distance(start);
    }
    let along = ((point - start).dot(segment) / length_squared).clamp(0.0, 1.0);
    let closest = start + segment * along;
    point.distance(closest)
}

/// `LineIntersectsRect`: whether the straight line between a connection's
/// ends crosses or lies in the rectangle.
fn segment_meets_rect(start: Pos2, end: Pos2, rect: Rect) -> bool {
    if rect.contains(start) || rect.contains(end) {
        return true;
    }
    let corners = [rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom()];
    for index in 0..4 {
        let edge_start = corners[index];
        let edge_end = corners[(index + 1) % 4];
        if segments_cross(start, end, edge_start, edge_end) {
            return true;
        }
    }
    false
}

fn segments_cross(first_start: Pos2, first_end: Pos2, second_start: Pos2, second_end: Pos2) -> bool {
    let side = |origin: Pos2, toward: Pos2, point: Pos2| (toward - origin).x * (point - origin).y - (toward - origin).y * (point - origin).x;
    let first_a = side(first_start, first_end, second_start);
    let first_b = side(first_start, first_end, second_end);
    let second_a = side(second_start, second_end, first_start);
    let second_b = side(second_start, second_end, first_end);
    first_a * first_b < 0.0 && second_a * second_b < 0.0
}

/// Points around a rounded rectangle (for dashed borders).
fn rounded_outline(rect: Rect, radius: f32) -> Vec<Pos2> {
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (Pos2::new(rect.right() - radius, rect.top() + radius), -90.0f32),
        (Pos2::new(rect.right() - radius, rect.bottom() - radius), 0.0),
        (Pos2::new(rect.left() + radius, rect.bottom() - radius), 90.0),
        (Pos2::new(rect.left() + radius, rect.top() + radius), 180.0),
    ];
    let mut points = vec![Pos2::new(rect.left() + radius, rect.top())];
    for (center, start_angle) in corners {
        for step in 0..=4 {
            let angle = (start_angle + step as f32 * 22.5).to_radians();
            points.push(center + Vec2::new(angle.cos(), angle.sin()) * radius);
        }
    }
    points.push(Pos2::new(rect.left() + radius, rect.top()));
    points
}

/// A border as WPF strokes it: solid, or dashed with the dash array in
/// stroke widths (`1 2`, `2 4`, `4 6`).
fn border(painter: &egui::Painter, rect: Rect, radius: f32, stroke: Stroke, dashes: Option<(f32, f32)>) {
    match dashes {
        None => {
            painter.rect_stroke(rect, radius, stroke, StrokeKind::Inside);
        }
        Some((dash, gap)) => {
            let inset = rect.shrink(stroke.width / 2.0);
            let outline = rounded_outline(inset, radius);
            let shapes = Shape::dashed_line(&outline, stroke, dash * stroke.width, gap * stroke.width);
            painter.extend(shapes);
        }
    }
}

/// The groups of the shown tab and, for each node and note a collapsed
/// group hides, that group's box.
struct GroupFrame {
    groups: Vec<GroupData>,
    hidden_in: HashMap<i32, Rect>,
}

fn group_frame(editor: &Editor, tab: usize) -> GroupFrame {
    let all = editor.document.groups();
    let groups: Vec<GroupData> = all.into_iter().filter(|group| group.graph_index == tab).collect();
    let mut hidden_in = HashMap::new();
    for group in groups.iter().filter(|group| group.collapsed) {
        for child in &group.children {
            hidden_in.insert(*child, group.bounds());
        }
    }
    GroupFrame { groups, hidden_in }
}

pub fn show(ui: &mut Ui, rect: Rect, editor: &mut Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    // Everything the graph draws or hit-tests keeps to its rectangle:
    // icons and rename boxes drawn through `ui` clip there as the painter
    // does, and a node, group or note panned past the edge takes no
    // clicks beyond it.
    let outer_clip = ui.clip_rect();
    ui.shrink_clip_rect(rect);
    state.load_views(editor);
    state.follow_restored_views(editor);
    state.reconcile_tabs(editor);
    state.rect = rect;
    take_shines(ui, editor, state);
    let hover = ui.input(|input| input.pointer.hover_pos());
    state.pointer = hover.filter(|point| rect.contains(*point));
    state.shift_held = ui.input(|input| input.modifiers.shift);
    state.snap_to_ports = editor.user.options.snap_to_ports();
    let origin = rect.min;
    let painter = ui.painter_at(rect);

    // Lay out this tab's nodes, less those collapsed groups hide.
    let tab = state.tab;
    let frame = group_frame(editor, tab);
    let default_size = editor.user.options.default_node_size().to_string();
    let tab_nodes = editor.document.terrain().nodes.values().filter(|node| node_tab(node) == tab);
    let shown_nodes = tab_nodes.filter(|node| !frame.hidden_in.contains_key(&node.id));
    let mut layouts: Vec<NodeLayout> = shown_nodes.map(|node| layout_node(node, &default_size)).collect();
    name_portals(&mut layouts, editor);
    layouts.sort_by_key(|layout| editor.selection.contains(&layout.id));
    let by_id: HashMap<i32, usize> = layouts.iter().enumerate().map(|(index, layout)| (layout.id, index)).collect();

    apply_zoom_request(state, &layouts, &frame, editor, rect);

    // The background takes clicks, drags and the context menu.
    let background_id = ui.id().with("graph-background");
    let background = ui.interact(rect, background_id, Sense::click_and_drag());
    let pan_started = handle_pan_and_zoom(ui, &background, state, rect);
    // Record Graph Movements and Zoom in Undos: a pan begins an undo step,
    // the view as it was (`GraphModified("Graph panned")`).
    if pan_started && editor.user.options.record_graph_movements() {
        editor.document.checkpoint("Graph panned");
    }
    auto_pan(ui, state, rect);

    let view = state.view();
    let zoom = view.zoom;
    if state.show_grid {
        draw_grid(&painter, rect, &view);
    }
    if let Some(Drag::Push { press, .. }) = &state.drag {
        let pointer = ui.input(|input| input.pointer.interact_pos());
        draw_push_band(&painter, rect, &view, origin, *press, pointer);
    }

    let connections = layout_connections(editor, &layouts, &by_id, &frame, &view, origin, tab);
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let pointer_on_graph = pointer.filter(|point| rect.contains(*point));
    // The distance test is geometric and blind to layers, so a connection
    // behind an open menu (the graph's own, the Lazy Menu, any popup) would
    // highlight through it and have `handle_background` read its spot as a
    // connection. Egui gives hover to the topmost widget only: a menu over
    // the pointer means the background is not hovered.
    let hovered_connection = pointer_on_graph
        .filter(|_| state.drag.is_none())
        .filter(|_| background.hovered())
        .and_then(|point| {
            connections.iter().position(|connection| connection.points.windows(2).any(|segment| distance_to_segment(point, segment[0], segment[1]) < 5.0))
        });
    for (index, connection) in connections.iter().enumerate() {
        let selected = state.selected_connections.iter().any(|(to, port)| *to == connection.to && *port == connection.to_port);
        let hovered = hovered_connection == Some(index);
        let width = if selected || hovered { 3.0 } else { 2.0 };
        let stroke = PathStroke::new((width * zoom).max(1.0), connection.color);
        let path = PathShape::line(connection.points.clone(), stroke);
        painter.add(path);
    }

    // Groups (`ZIndex` 40) over the connections, then notes (45), then
    // the nodes.
    graph_groups::show(ui, &painter, editor, state, &frame.groups, origin, commands);
    let hidden_elements: HashSet<i32> = frame.hidden_in.keys().copied().collect();
    crate::ui::graph_notes::show(ui, &painter, editor, state, origin, &hidden_elements, commands);

    // Nodes, selected ones on top.
    let mut node_responses: Vec<(usize, Response)> = Vec::new();
    let mut port_responses: Vec<(usize, PortLayout, Response)> = Vec::new();
    let snap_target = pointer_on_graph.and_then(|point| snapping_port(&layouts, &view, origin, state, point));
    for (index, layout) in layouts.iter().enumerate() {
        let screen = view.rect_to_screen(origin, layout.rect);
        if !screen.intersects(rect.expand(40.0)) {
            continue;
        }
        let response = ui.interact(screen, ui.id().with(("node", layout.id)), Sense::click_and_drag());
        if state.dimmed.contains(&layout.id) {
            // `TVINode.UpdateNodeCriteria`: `NodeElement.Opacity` 0.5.
            let mut dimmed_ui = ui.new_child(UiBuilder::new());
            dimmed_ui.multiply_opacity(0.5);
            let dimmed_painter = dimmed_ui.painter_at(rect);
            draw_node(&dimmed_ui, &dimmed_painter, editor, layout, screen, zoom);
        } else {
            draw_node(ui, &painter, editor, layout, screen, zoom);
        }
        node_responses.push((index, response));
        for port in layout.ports() {
            let [element_area, grid_area] = port.hit.map(|area| view.rect_to_screen(origin, area));
            if !element_area.union(grid_area).intersects(rect) {
                continue;
            }
            let id = ui.id().with(("port", layout.id, &port.name, port.is_input));
            let element_response = ui.interact(element_area, id, Sense::click_and_drag());
            let grid_response = ui.interact(grid_area, id.with("hit-grid"), Sense::click_and_drag());
            let response = element_response.union(grid_response);
            if response.hovered() {
                ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
            }
            let response = if state.popup.is_none() {
                let tooltip = port_tooltip(editor, layout.id, port);
                response.tooltip_at_pointer(tooltip)
            } else {
                response
            };
            port_responses.push((index, port.clone(), response));
        }
        draw_ports(&painter, layout, &view, origin, state, pointer_on_graph, snap_target.as_ref());
    }
    draw_shines(ui, &painter, state, &layouts, &view, origin);

    handle_port_drags(ui, editor, state, &layouts, &frame.groups, &port_responses, origin, commands);
    handle_node_interaction(ui, editor, state, &layouts, &frame.groups, &node_responses, origin, commands);
    handle_background(ui, editor, state, &layouts, &frame.groups, &background, &connections, hovered_connection, origin, commands);
    handle_drops(ui, editor, state, &layouts, commands);
    draw_drag_overlay(ui, &painter, state, &layouts, origin, pointer);
    rename_overlay(ui, state, &layouts, origin, commands);
    graph_groups::rename_overlay(ui, editor, state, &frame.groups, origin);
    crate::ui::graph_popups::show(ui, editor, state, commands);
    ui.set_clip_rect(outer_clip);

    // Remember the view for saving.
    let view = state.view();
    editor.document.store_view_quietly(state.tab, [view.location.x, view.location.y], view.zoom as f64);
}

/// Every connection into this tab's nodes from this tab's nodes. One out
/// of a collapsed group starts at the group's right side, one into it ends
/// at its left (`GroupExit`, `GroupEnter`); those inside it are not drawn.
fn layout_connections(editor: &Editor, layouts: &[NodeLayout], by_id: &HashMap<i32, usize>, frame: &GroupFrame, view: &GraphView, origin: Pos2, tab: usize) -> Vec<ConnectionLayout> {
    let mut connections = Vec::new();
    let nodes = &editor.document.terrain().nodes;
    for node in nodes.values().filter(|node| node_tab(node) == tab) {
        let target_box = frame.hidden_in.get(&node.id);
        let target_layout = by_id.get(&node.id).map(|index| &layouts[*index]);
        for port in &node.ports {
            let Some(record) = &port.record else { continue };
            let Some(source) = nodes.get(&record.from).filter(|source| node_tab(source) == tab) else { continue };
            let source_box = frame.hidden_in.get(&source.id);
            if let (Some(source_box), Some(target_box)) = (source_box, target_box)
                && source_box == target_box
            {
                continue;
            }
            let source_layout = by_id.get(&source.id).map(|index| &layouts[*index]);
            let source_port = source_layout.and_then(|layout| layout.port(&record.from_port, false));
            let target_port = target_layout.and_then(|layout| layout.port(&port.name, true));
            // Portal to portal: no line (`RedrawConnections`).
            let source_publishes = source_port.is_some_and(|port| port.portal == Some(Portal::Publishing));
            let target_consumes = target_port.is_some_and(|port| port.portal == Some(Portal::Consuming));
            if source_publishes && target_consumes {
                continue;
            }
            let start = match (source_box, source_port) {
                (Some(group_box), _) => Pos2::new(group_box.right(), group_box.center().y),
                (None, Some(source_port)) => source_port.anchor,
                (None, None) => continue,
            };
            let end = match (target_box, target_port) {
                (Some(group_box), _) => Pos2::new(group_box.left(), group_box.center().y),
                (None, Some(target_port)) => target_port.anchor,
                (None, None) => continue,
            };
            let screen_start = view.to_screen(origin, start);
            let screen_end = view.to_screen(origin, end);
            let standard = target_layout.is_some_and(|layout| layout.size == NodeSize::Standard);
            let points = circuit_points(screen_start, screen_end, standard);
            let color = node_color(source);
            let connection = ConnectionLayout {
                to: node.id,
                to_port: port.name.clone(),
                points,
                color,
            };
            connections.push(connection);
        }
    }
    connections
}

/// The screen ends of the connection into `to_port` (`StartPoint`,
/// `EndPoint`), when both nodes show on the graph.
pub fn connection_ends(editor: &Editor, state: &GraphState, to: i32, to_port: &str) -> Option<(Pos2, Pos2)> {
    let target = editor.document.node(to)?;
    let record = target.port(to_port)?.record.clone()?;
    let source = editor.document.node(record.from)?;
    let default_size = editor.user.options.default_node_size().to_string();
    let source_layout = layout_node(source, &default_size);
    let target_layout = layout_node(target, &default_size);
    let start = source_layout.port(&record.from_port, false)?.anchor;
    let end = target_layout.port(to_port, true)?.anchor;
    let view = state.view();
    let origin = state.rect.min;
    Some((view.to_screen(origin, start), view.to_screen(origin, end)))
}

fn apply_zoom_request(state: &mut GraphState, layouts: &[NodeLayout], frame: &GroupFrame, editor: &Editor, rect: Rect) {
    let Some(request) = state.zoom_request.take() else {
        return;
    };
    let tab = state.tab;
    let fit = fit_rect(state, rect);
    let view = state.view_mut();
    if request == ZoomRequest::Reset {
        let center = view.location + rect.size() / (2.0 * view.zoom);
        view.zoom = 1.0;
        view.location = center - rect.size() / 2.0;
        return;
    }
    let centred = match request {
        ZoomRequest::Center(id) => layouts.iter().find(|layout| layout.id == id).map(|layout| layout.rect),
        ZoomRequest::CenterNote(id) => crate::ui::graph_notes::note_rect(editor, tab, id),
        ZoomRequest::CenterGroup(id) => frame.groups.iter().find(|group| group.id == id).map(GroupData::bounds),
        _ => None,
    };
    if let Some(element) = centred {
        view.location = element.center().to_vec2() - (fit.center() - rect.min) / view.zoom;
        return;
    }
    if matches!(request, ZoomRequest::Center(_) | ZoomRequest::CenterNote(_) | ZoomRequest::CenterGroup(_)) {
        return;
    }
    // `ZoomExtents` frames the tab's nodes, groups and notes, those
    // collapsed groups hide too; `ZoomSelection` the selected nodes. Both
    // go by the elements' top-left corners.
    let tab_nodes = editor.document.terrain().nodes.values().filter(|node| node_tab(node) == tab);
    let corners: Vec<Pos2> = match request {
        ZoomRequest::Selection => {
            let selected = tab_nodes.filter(|node| editor.selection.contains(&node.id));
            selected.map(|node| Pos2::new(node.position[0], node.position[1])).collect()
        }
        _ => {
            let nodes = tab_nodes.map(|node| Pos2::new(node.position[0], node.position[1]));
            let groups = frame.groups.iter().map(|group| group.bounds().min);
            let notes = crate::ui::graph_notes::note_rects(editor, tab).into_iter().map(|(_, note)| note.min);
            nodes.chain(groups).chain(notes).collect()
        }
    };
    if corners.is_empty() {
        return;
    }
    let framed = Rect::from_points(&corners).expand(EXTENTS_MARGIN + FIT_MARGIN);
    // Fit the visible strip beside the toolbox so navigation leaves the
    // framed elements in sight in narrow layouts too.
    let fitted = (fit.width() / framed.width()).min(fit.height() / framed.height());
    if fitted >= MIN_ZOOM {
        view.zoom = fitted.min(MAX_ZOOM);
    }
    view.location = framed.center().to_vec2() - (fit.center() - rect.min) / view.zoom;
}

fn fit_rect(state: &GraphState, rect: Rect) -> Rect {
    if !state.toolbox_rect.intersects(rect) {
        return rect;
    }
    let left = (state.toolbox_rect.right() + 12.0).min(rect.right() - 40.0).max(rect.left());
    Rect::from_min_max(Pos2::new(left, rect.top()), rect.max)
}

/// Pans with the middle button (anywhere on the graph) or a right drag on
/// its background, and zooms with the wheel. Returns whether a pan began
/// (the middle button pressed on the graph, as `PanButton` goes down, or
/// a right drag started).
fn handle_pan_and_zoom(ui: &Ui, background: &Response, state: &mut GraphState, rect: Rect) -> bool {
    let pointer = ui.input(|input| input.pointer.hover_pos());
    let over = pointer.is_some_and(|point| rect.contains(point));
    let over_toolbox = pointer.is_some_and(|point| state.toolbox_rect.contains(point));
    let on_graph_layer = ui.rect_contains_pointer(rect);
    let blocked = ui.ctx().any_popup_open() || !ui.ctx().memory(|memory| memory.is_above_modal_layer(ui.layer_id()));
    let accepts_input = over && !over_toolbox && on_graph_layer && !blocked;
    let middle_pressed = ui.input(|input| input.pointer.button_pressed(PointerButton::Middle));
    if middle_pressed {
        state.middle_panning = accepts_input;
    }
    let middle_held = ui.input(|input| input.pointer.button_down(PointerButton::Middle));
    if !middle_held || blocked {
        state.middle_panning = false;
    }
    let secondary_panning = !blocked && background.dragged_by(PointerButton::Secondary);
    let pan_started = (middle_pressed && state.middle_panning) || (!blocked && background.drag_started_by(PointerButton::Secondary));
    if state.middle_panning || secondary_panning {
        let delta = if state.middle_panning {
            ui.input(|input| input.pointer.delta())
        } else {
            background.drag_delta()
        };
        let view = state.view_mut();
        view.location -= delta / view.zoom;
        ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
    }
    if !accepts_input {
        return pan_started;
    }
    let (scroll, pinch) = ui.input(|input| (input.smooth_scroll_delta.y, input.zoom_delta()));
    let factor = (scroll * 0.0025).exp() * pinch;
    if (factor - 1.0).abs() < 1e-4 {
        return pan_started;
    }
    let Some(point) = pointer else {
        return pan_started;
    };
    let view = state.view_mut();
    let before = view.to_canvas(rect.min, point);
    view.zoom = (view.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
    let after_offset = (point - rect.min) / view.zoom;
    view.location = before.to_vec2() - after_offset;
    pan_started
}

/// `HandleAutoPanning`, on each move of the pointer while nodes, a group
/// or a note are moved with it: [`auto_pan_step`] for each.
fn auto_pan(ui: &Ui, state: &mut GraphState, rect: Rect) {
    let moving = matches!(state.drag, Some(Drag::Elements { .. })) || state.group_drag.is_some_and(|drag| drag.moves()) || state.note_drag.is_some_and(|drag| drag.moves());
    let held = ui.input(|input| input.pointer.primary_down());
    if !moving || !held {
        return;
    }
    let moves: Vec<Pos2> = ui.input(|input| {
        let positions = input.events.iter().filter_map(|event| match event {
            egui::Event::PointerMoved(position) => Some(*position),
            _ => None,
        });
        positions.collect()
    });
    for pointer in moves {
        auto_pan_step(state.view_mut(), rect, pointer);
    }
}

/// One `HandleAutoPanning` step with the pointer at `pointer` (screen):
/// within `AUTO_PAN_EDGE` of a side of the graph `rect`, or past it, the
/// view moves `AUTO_PAN_SPEED / (zoom * 2)` canvas units that way on each
/// axis; then it is kept on the canvas, the graph's size taken in screen
/// points rather than canvas units, as Gaea does.
fn auto_pan_step(view: &mut GraphView, rect: Rect, pointer: Pos2) {
    let step = AUTO_PAN_SPEED / (view.zoom * 2.0);
    let local = pointer - rect.min;
    let mut location = view.location;
    if local.x <= AUTO_PAN_EDGE {
        location.x -= step;
    } else if local.x >= rect.width() - AUTO_PAN_EDGE {
        location.x += step;
    }
    if local.y <= AUTO_PAN_EDGE {
        location.y -= step;
    } else if local.y >= rect.height() - AUTO_PAN_EDGE {
        location.y += step;
    }
    location = location.max(Vec2::ZERO);
    if location.x + rect.width() > CANVAS_EXTENTS {
        location.x = CANVAS_EXTENTS - rect.width();
    }
    if location.y + rect.height() > CANVAS_EXTENTS {
        location.y = CANVAS_EXTENTS - rect.height();
    }
    view.location = location;
}

/// `SnapElementToGrid`: to the nearer multiple of the unit, halves down
/// (the remainder keeps the position's sign, as .NET's `%` does).
fn snap_to_unit(position: f32, unit: f32) -> f32 {
    let offset = position % unit;
    let half = unit * 0.5;
    let shift = if offset <= half { -offset } else { unit - offset };
    position + shift
}

fn draw_grid(painter: &egui::Painter, rect: Rect, view: &GraphView) {
    let levels = [(10.0, theme::GRID_FINE), (100.0, theme::GRID_COARSE), (500.0, theme::GRID_MEGA)];
    for (spacing, color) in levels {
        let screen_spacing = spacing * view.zoom;
        if screen_spacing < 6.0 {
            continue;
        }
        let stroke = Stroke::new(1.0, color);
        let spacing = spacing as f64;
        let zoom = view.zoom as f64;
        for axis in 0..2 {
            let location = view.location[axis] as f64;
            let extent = rect.size()[axis] as f64;
            if !location.is_finite() || !extent.is_finite() || !zoom.is_finite() {
                continue;
            }
            let offset = -location.rem_euclid(spacing) * zoom;
            let count = (extent / screen_spacing as f64).ceil() as usize + 2;
            for index in 0..count {
                let coordinate = offset + index as f64 * screen_spacing as f64;
                let line = if axis == 0 {
                    let x = rect.left() + coordinate as f32;
                    [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())]
                } else {
                    let y = rect.top() + coordinate as f32;
                    [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)]
                };
                painter.line_segment(line, stroke);
            }
        }
    }
}

/// How the node's build state shows on its main border: colour, dashes
/// (in stroke widths) and the title colour it forces.
/// The outline a node's state gives it (the `NodeState` triggers on
/// `MainBorder`), its dashes and the colour its name and icon take. Built
/// and building nodes keep the node style's own outline (`None`).
fn state_border(state: NodeState, building: bool) -> (Option<Color32>, Option<(f32, f32)>, Option<Color32>) {
    if building {
        return (None, None, None);
    }
    match state {
        NodeState::Built => (None, None, None),
        NodeState::Unbuilt => (Some(theme::NODE_BORDER_SELECTED), Some((1.0, 2.0)), None),
        NodeState::Error => (Some(theme::ACCENT_RED), Some((1.0, 2.0)), Some(theme::ACCENT_STRONG_RED)),
        NodeState::Bypassed => (Some(theme::ACCENT_PURPLE), Some((2.0, 4.0)), Some(theme::ACCENT_MUTED_PURPLE)),
        NodeState::Baked => (Some(theme::ACCENT_MUTED_GRAY), Some((1.0, 2.0)), None),
    }
}

/// How high a line of `font` lays out: whole pixels, as the names drawn
/// and typed in it are, which the font's own row height needn't be.
pub(crate) fn line_height(ctx: &egui::Context, font: &egui::FontId) -> f32 {
    ctx.fonts_mut(|fonts| fonts.layout_no_wrap(String::new(), font.clone(), Color32::PLACEHOLDER).size().y)
}

/// Where `draw_node` writes a node's name on `screen` at `zoom`, and in
/// what font (`PART_MainContent`): the line it starts on, as wide as it
/// runs before it is cut short (its `MaxWidth`, or what its margins leave
/// of the node). A standard node's starts 50, 14 in; a compact node's 80
/// in, centred down the node; a small node's 4 in, its foot 16 below the
/// box. Zoomed out (simplified) the name keeps its place in a 24 font.
/// None on a small node with a Mask port: the name gives way to it.
fn title_line(ctx: &egui::Context, layout: &NodeLayout, screen: Rect, zoom: f32) -> Option<(Rect, egui::FontId)> {
    if layout.size == NodeSize::Small && layout.mask.is_some() {
        return None;
    }
    let simplified = zoom < SIMPLIFIED_ZOOM;
    let size = match layout.size {
        _ if simplified => 24.0,
        NodeSize::Standard | NodeSize::Compact => 13.0,
        NodeSize::Small => 10.0,
    };
    let font = theme::semibold(size * zoom);
    let height = line_height(ctx, &font);
    let line = match layout.size {
        NodeSize::Standard => Rect::from_min_size(screen.min + Vec2::new(50.0, 14.0) * zoom, Vec2::new(100.0 * zoom, height)),
        NodeSize::Compact => {
            let left = screen.left() + 80.0 * zoom;
            Rect::from_min_size(Pos2::new(left, screen.center().y - height / 2.0), Vec2::new((COMPACT_WIDTH - 105.0) * zoom, height))
        }
        NodeSize::Small => {
            let top_left = Pos2::new(screen.left() + 4.0 * zoom, screen.bottom() + 16.0 * zoom - height);
            Rect::from_min_size(top_left, Vec2::new((SMALL_SIZE - 4.0) * zoom, height))
        }
    };
    Some((line, font))
}

fn draw_node(ui: &Ui, painter: &egui::Painter, editor: &Editor, layout: &NodeLayout, screen: Rect, zoom: f32) {
    let Some(node) = editor.document.node(layout.id) else {
        return;
    };
    let status = editor.status(layout.id);
    let bypassed = node.base.get("StateOverride").and_then(Value::as_str) == Some("Bypassed");
    let state = if bypassed { NodeState::Bypassed } else { status.state };
    let selected = editor.selection.contains(&layout.id);
    let (state_color, dashes, forced_title) = state_border(state, status.building);
    let mut title_color = forced_title.unwrap_or(layout.color);
    if state == NodeState::Bypassed {
        title_color = title_color.gamma_multiply(0.8);
    }
    // The node style (`UseModernTemplate`): Muted outlines nodes in
    // `Menu-Border` and a chosen one in its colour on #0F1011; Vibrant
    // outlines them in their colour and a chosen one in the primary accent
    // on #050505.
    let vibrant = editor.user.options.vibrant_nodes();
    let style_border = if vibrant { layout.color } else { theme::MENU_BORDER };
    let selection_border = if vibrant { theme::ACCENT_PRIMARY } else { layout.color };
    let selected_fill = if vibrant { theme::NODE_BACKGROUND_SELECTED_VIBRANT } else { theme::NODE_BACKGROUND_SELECTED };
    let radius = 4.0 * zoom;
    let fill = if status.building {
        theme::NODE_BACKGROUND_BUILDING
    } else if selected {
        selected_fill
    } else {
        theme::NODE_BACKGROUND
    };
    painter.rect_filled(screen, radius, fill);
    let stroke_width = (2.0 * zoom).max(1.0);
    let main_color = if selected { theme::NODE_BORDER_SELECTED } else { state_color.unwrap_or(style_border) };
    border(painter, screen, radius, Stroke::new(stroke_width, main_color), dashes);
    if selected {
        let selection_stroke = Stroke::new(stroke_width, selection_border);
        painter.rect_stroke(screen, radius, selection_stroke, StrokeKind::Inside);
    }

    // Progress line while building (`PART_Progress`; margin 6 and 3 high
    // when simplified).
    let simplified = zoom < SIMPLIFIED_ZOOM;
    if status.building {
        let (margin, height) = if simplified { (6.0, 3.0) } else { (3.0, 1.0) };
        building_bar(ui, painter, screen, zoom, margin, height, layout.color);
    }

    // `PART_Icon`, hidden when simplified.
    if !simplified {
        let icon_offset = match layout.size {
            NodeSize::Standard => Vec2::new(18.0, 14.0),
            NodeSize::Compact => Vec2::new(45.0, (COMPACT_HEIGHT - 20.0) / 2.0),
            NodeSize::Small => Vec2::new((SMALL_SIZE - 20.0) / 2.0, (SMALL_SIZE - 20.0) / 2.0),
        };
        let icon_rect = Rect::from_min_size(screen.min + icon_offset * zoom, Vec2::splat(20.0 * zoom));
        icons::paint(ui, icon_rect, &layout.icon, title_color);
    }

    if let Some((title, title_font)) = title_line(painter.ctx(), layout, screen, zoom) {
        let galley = elided_galley(painter, &node.name, title_font, title_color, title.width());
        painter.galley(title.min, galley, title_color);
    }
    // Simplified, the icon stack hides too.
    if simplified {
        return;
    }

    // `PART_IconStack` (margin 0, 3, 1, 3), right-aligned at the bottom:
    // the build time (monospace 10, `ParamsN-Color6`, margin 4,
    // `Options.ShowBuildTime`), then locked, then marked for export.
    let mut icons_shown: Vec<(&str, Color32, f32)> = Vec::new();
    let locked = editor.preview_locked && editor.preview_node == Some(layout.id);
    if locked {
        icons_shown.push(("photo-sensor-3", theme::ACCENT_GREEN, 16.0));
    }
    if node.base.get("SaveDefinition").is_some_and(|value| !value.is_null()) {
        icons_shown.push(("floppy", theme::ACCENT_PRIMARY, 14.0));
    }
    let mut right = screen.right() - 1.0 * zoom;
    let stack_bottom = screen.bottom() - 3.0 * zoom;
    // `Options.ShowBuildTime`: without it the time is an empty text.
    let time_text = if editor.user.options.show_build_time() { format!("{:.2}s", status.milliseconds / 1000.0) } else { String::new() };
    let time_galley = painter.layout_no_wrap(time_text, theme::monospace(10.0 * zoom), BUILD_TIME_COLOR);
    // The stack is as tall as the time with its margins; icons centre in it.
    let stack_height = time_galley.size().y + 8.0 * zoom;
    let stack_center = stack_bottom - stack_height / 2.0;
    for (icon, color, size) in icons_shown.into_iter().rev() {
        let icon_size = size * zoom;
        let icon_rect = Rect::from_min_size(Pos2::new(right - 2.0 * zoom - icon_size, stack_center - icon_size / 2.0), Vec2::splat(icon_size));
        icons::paint(ui, icon_rect, icon, color);
        right -= icon_size + 2.0 * zoom;
    }
    let time_position = Pos2::new(right - 4.0 * zoom - time_galley.size().x, stack_center - time_galley.size().y / 2.0);
    if time_position.x >= screen.left() + 3.0 * zoom {
        painter.galley(time_position, time_galley, BUILD_TIME_COLOR);
    }
}

/// `ParamsN-Color6`: a node's build time.
const BUILD_TIME_COLOR: Color32 = Color32::from_rgb(0x49, 0x50, 0x57);

/// The node's tooltip (`PART_MainGrid.ToolTip`, at the pointer): its name
/// in semibold; "Build time:" and its time (`MillisecondsToTimeSpan`, the
/// run's own space and the one between the runs before it); "Status:",
/// its state in semibold (`EnumToString`, title case) and its error in
/// `Accent-Orange`.
fn node_tooltip(editor: &Editor, id: i32) -> egui::text::LayoutJob {
    let status = editor.status(id);
    let node = editor.document.node(id);
    let name = node.map(|node| node.name.clone()).unwrap_or_default();
    let bypassed = node.is_some_and(|node| node.base.get("StateOverride").and_then(Value::as_str) == Some("Bypassed"));
    let state = if status.building {
        "Is Building"
    } else if bypassed {
        "Bypassed"
    } else {
        match status.state {
            NodeState::Unbuilt => "Unbuilt",
            NodeState::Built => "Built",
            NodeState::Error => "Error",
            NodeState::Bypassed => "Bypassed",
            NodeState::Baked => "Baked",
        }
    };
    let time = crate::ui::data_view::stats::humanize_milliseconds(status.milliseconds);
    let error = status.error.unwrap_or_default();
    let plain = egui::TextFormat::simple(theme::interface(12.0), theme::LABEL_EMPHASIS);
    let bold = egui::TextFormat::simple(theme::semibold(12.0), theme::LABEL_EMPHASIS);
    let warning = egui::TextFormat::simple(theme::interface(12.0), theme::ACCENT_ORANGE);
    let mut job = egui::text::LayoutJob::default();
    job.append(&name, 0.0, bold.clone());
    job.append("\nBuild time:  ", 0.0, plain.clone());
    job.append(&time, 0.0, plain.clone());
    job.append("\nStatus: ", 0.0, plain.clone());
    job.append(state, 0.0, bold);
    job.append(" ", 0.0, plain);
    job.append(&error, 0.0, warning);
    job
}

/// How long a shine lasts (`ShineBoard`'s longest animation).
const SHINE_SECONDS: f64 = 0.5;
/// The band's opacity at its peak (`#60FFE01E`).
const SHINE_PEAK: f32 = 0x60 as f32 / 255.0;

/// `NodeElement.Shine`: the nodes the editor asks to shine start now,
/// unless Show shine effect on Node when build completes is off, a gizmo
/// is on (`O.IsShineDisabled`) or the interface's animations are off.
fn take_shines(ui: &Ui, editor: &mut Editor, state: &mut GraphState) {
    let asked = std::mem::take(&mut editor.shines);
    let options = &editor.user.options;
    if asked.is_empty() || !options.build_shine() || options.disable_animations() || editor.gizmo_active {
        return;
    }
    let now = ui.input(|input| input.time);
    for id in asked {
        state.shines.insert(id, now);
    }
}

/// `BuildShine` (`ShineRect`: margin 1, corners 4, over everything the
/// node shows): the node's colour seen through a band (`OpacityMask`:
/// transparent, #60, transparent) that sweeps across it left to right.
fn draw_shines(ui: &Ui, painter: &egui::Painter, state: &mut GraphState, layouts: &[NodeLayout], view: &GraphView, origin: Pos2) {
    if state.shines.is_empty() {
        return;
    }
    let now = ui.input(|input| input.time);
    state.shines.retain(|_, started| now - *started < SHINE_SECONDS);
    for layout in layouts {
        let Some(started) = state.shines.get(&layout.id) else {
            continue;
        };
        let stops = shine_stops((now - started) as f32);
        let screen = view.rect_to_screen(origin, layout.rect).shrink(view.zoom);
        let mesh = shine_mesh(screen, 4.0 * view.zoom, layout.color, stops);
        painter.add(Shape::mesh(mesh));
    }
    ui.ctx().request_repaint();
}

/// The band's stops `seconds` in, across the node (`ShineBoard`): from
/// -0.65, -0.5 and -0.49 to 1.25 (over 0.5 s), 1.3 and 1.31 (0.45 s).
fn shine_stops(seconds: f32) -> [f32; 3] {
    let along = |from: f32, to: f32, duration: f32| from + (to - from) * (seconds / duration).min(1.0);
    [along(-0.65, 1.25, 0.5), along(-0.5, 1.3, 0.45), along(-0.49, 1.31, 0.45)]
}

/// The band's opacity at `x` across the node: rising from the first stop
/// to its peak at the second, falling to nothing at the third.
fn shine_opacity(x: f32, stops: [f32; 3]) -> f32 {
    let [rise, peak, fall] = stops;
    if x <= rise || x >= fall {
        return 0.0;
    }
    if x < peak {
        return SHINE_PEAK * (x - rise) / (peak - rise);
    }
    SHINE_PEAK * (fall - x) / (fall - peak)
}

/// The band over a rounded rectangle: vertical strips meeting at the
/// stops (so the opacity is exact between them) and stepping round the
/// corners.
fn shine_mesh(rect: Rect, radius: f32, color: Color32, stops: [f32; 3]) -> egui::Mesh {
    let width = rect.width().max(1.0);
    let radius = radius.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corner = radius / width;
    let mut columns: Vec<f32> = vec![0.0, 1.0];
    columns.extend(stops.iter().copied().filter(|stop| *stop > 0.0 && *stop < 1.0));
    for step in 1..6 {
        let along = corner * step as f32 / 6.0;
        columns.push(along);
        columns.push(1.0 - along);
    }
    columns.sort_by(f32::total_cmp);
    columns.dedup();
    // How far a corner's curve keeps the band from the top and bottom.
    let inset = |x: f32| {
        let from_edge = (x * width).min((1.0 - x) * width);
        if from_edge >= radius {
            return 0.0;
        }
        let across = radius - from_edge;
        radius - (radius * radius - across * across).max(0.0).sqrt()
    };
    let mut mesh = egui::Mesh::default();
    for (index, x) in columns.iter().enumerate() {
        let screen_x = rect.left() + x * width;
        let shade = color.gamma_multiply(shine_opacity(*x, stops));
        let edge = inset(*x);
        mesh.colored_vertex(Pos2::new(screen_x, rect.top() + edge), shade);
        mesh.colored_vertex(Pos2::new(screen_x, rect.bottom() - edge), shade);
        if index > 0 {
            let top = (index * 2) as u32;
            mesh.add_triangle(top - 2, top - 1, top);
            mesh.add_triangle(top - 1, top + 1, top);
        }
    }
    mesh
}

/// The thin animated line across a building node's top.
fn building_bar(ui: &Ui, painter: &egui::Painter, screen: Rect, zoom: f32, margin: f32, height: f32, color: Color32) {
    let time = ui.input(|input| input.time) as f32;
    let track = Rect::from_min_size(screen.min + Vec2::splat(margin * zoom), Vec2::new(screen.width() - 2.0 * margin * zoom, (height * zoom).max(1.0)));
    painter.rect_filled(track, 0.0, theme::PROGRESS_EMPTY);
    let phase = (time * 0.8).fract();
    let width = track.width() * 0.3;
    let left = track.left() + (track.width() + width) * phase - width;
    let bar = Rect::from_min_max(Pos2::new(left.max(track.left()), track.top()), Pos2::new((left + width).min(track.right()), track.bottom()));
    if bar.width() > 0.0 {
        painter.rect_filled(bar, 0.0, color);
    }
    ui.ctx().request_repaint();
}

/// Text cut with an ellipsis to `max_width` (`TextTrimming.CharacterEllipsis`).
pub(crate) fn elided_galley(painter: &egui::Painter, text: &str, font: egui::FontId, color: Color32, max_width: f32) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(text.to_string(), egui::TextFormat::simple(font, color));
    job.wrap = egui::text::TextWrapping {
        max_width,
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('…'),
    };
    painter.layout_job(job)
}

#[allow(clippy::too_many_arguments)]
fn draw_ports(painter: &egui::Painter, layout: &NodeLayout, view: &GraphView, origin: Pos2, state: &GraphState, pointer: Option<Pos2>, snap_target: Option<&(i32, String, bool)>) {
    let zoom = view.zoom;
    let simplified = zoom < SIMPLIFIED_ZOOM;

    let mut painter = painter.clone();
    if state.dimmed.contains(&layout.id) {
        painter.multiply_opacity(0.5);
    }
    let painter = &painter;
    for port in layout.ports() {
        let dot = view.rect_to_screen(origin, port.dot);
        let hovered = pointer.is_some_and(|point| port.takes(view.to_canvas(origin, point)));
        let snapping = snap_target.is_some_and(|(node, name, is_input)| *node == layout.id && *name == port.name && *is_input == port.is_input);
        let is_mask = layout.mask.as_ref().is_some_and(|mask| mask.name == port.name) && port.is_input;
        let base = if is_mask { theme::ACCENT_MUTED_MAGENTA } else { layout.color };
        let opacity = if port.connected {
            1.0
        } else if hovered || snapping {
            0.8
        } else {
            0.4
        };
        let fill = base.gamma_multiply(opacity);
        if let Some(portal) = port.portal {
            draw_portal(painter, port, portal, base, view, origin, simplified);
        }
        let dot_shape = if simplified { Rect::from_center_size(dot.center(), SIMPLIFIED_PORT * zoom) } else { dot };
        let corner = if simplified { 0.0 } else { dot.width() / 2.0 };
        painter.rect_filled(dot_shape, corner, fill);
        if snapping {
            let outline = if port.connected { theme::ACCENT_RED } else { theme::ACCENT_BLUE };
            painter.rect_stroke(dot_shape, corner, Stroke::new(2.0, outline), StrokeKind::Outside);
        }
        // Compact and small nodes hide their port labels (`HideLabel`).
        let label_hidden = layout.size != NodeSize::Standard || port.collapsed;
        if simplified || label_hidden || port.portal.is_some() {
            continue;
        }
        let label_color = if hovered || snapping { theme::LABEL_EMPHASIS } else { theme::PORT_LABEL };
        let font = theme::semibold(10.0 * zoom);
        let galley = painter.layout_no_wrap(port.label.clone(), font, label_color);
        let y = dot.center().y - galley.size().y / 2.0;
        let x = if port.is_input { dot.left() + 18.0 * zoom } else { dot.right() - 18.0 * zoom - galley.size().x };
        painter.galley(Pos2::new(x, y), galley, label_color);
    }
}

/// A portal port (`PortElement` with `IsPortal`): a pill in the port's
/// colour reaching out of the node with the friendly name, and a dark ring
/// around the dot.
fn draw_portal(painter: &egui::Painter, port: &PortLayout, portal: Portal, color: Color32, view: &GraphView, origin: Pos2, simplified: bool) {
    let zoom = view.zoom;
    let element = view.rect_to_screen(origin, port.element);
    let font = theme::interface(11.0 * zoom);
    let (text_left, text_right, available) = match portal {
        Portal::Consuming => (6.0, 16.0, 114.0),
        Portal::Publishing => (18.0, 6.0, 75.0),
    };
    let text_width = painter.layout_no_wrap(port.portal_name.clone(), font.clone(), theme::PORTAL_FOREGROUND).size().x;
    let width = (text_width + (text_left + text_right) * zoom).clamp(36.0 * zoom, available * zoom);
    let height = 18.0 * zoom;
    let center_y = element.center().y;
    let pill = match portal {
        Portal::Consuming => {
            let right = element.left() + 14.0 * zoom;
            Rect::from_min_max(Pos2::new(right - width, center_y - height / 2.0), Pos2::new(right, center_y + height / 2.0))
        }
        Portal::Publishing => {
            let left = element.left() + 5.0 * zoom;
            Rect::from_min_max(Pos2::new(left, center_y - height / 2.0), Pos2::new(left + width, center_y + height / 2.0))
        }
    };
    let radius = if simplified { 0.0 } else { 9.0 * zoom };
    painter.rect_filled(pill, radius, color);
    painter.rect_stroke(pill, radius, Stroke::new(2.0 * zoom, theme::NODE_BACKGROUND), StrokeKind::Inside);
    let dot_center = view.rect_to_screen(origin, port.dot).center();
    painter.circle_filled(dot_center, 5.0 * zoom, theme::NODE_BACKGROUND);
    if simplified {
        return;
    }
    let name_width = pill.width() - (text_left + text_right) * zoom;
    let galley = elided_galley(painter, &port.portal_name, font, theme::PORTAL_FOREGROUND, name_width.max(1.0));
    let position = Pos2::new(pill.left() + text_left * zoom, center_y - galley.size().y / 2.0);
    painter.galley(position, galley, theme::PORTAL_FOREGROUND);
}

/// The port a dragged connection would be let go on (`HandleSnapToPorts`):
/// any port of another node, outputs as well as inputs. Let go outside
/// the graph, it is none: a port past the edge is not there to take it.
fn snapping_port(layouts: &[NodeLayout], view: &GraphView, origin: Pos2, state: &GraphState, point: Pos2) -> Option<(i32, String, bool)> {
    if !state.rect.contains(point) {
        return None;
    }
    let source_node = match &state.drag {
        Some(Drag::FromOutput { node, .. } | Drag::FromInput { node, .. }) => *node,
        _ => return None,
    };
    let mut best: Option<(f32, (i32, String, bool))> = None;
    for (index, layout) in layouts.iter().enumerate() {
        if layout.id == source_node {
            continue;
        }
        for port in layout.ports() {
            let point_canvas = view.to_canvas(origin, point);
            let covered = layouts[index + 1..].iter().any(|above| above.rect.contains(port.dot.center()) || above.rect.contains(point_canvas));
            if covered {
                continue;
            }
            let anchor = view.to_screen(origin, port.dot.center());
            let distance = anchor.distance(point);
            let near = if state.snap_to_ports { distance <= SNAP_DISTANCE.max(12.0 * view.zoom) } else { port.takes(view.to_canvas(origin, point)) };
            if !near {
                continue;
            }
            let better = best.as_ref().is_none_or(|(best_distance, _)| distance < *best_distance);
            if better {
                best = Some((distance, (layout.id, port.name.clone(), port.is_input)));
            }
        }
    }
    best.map(|(_, port)| port)
}

/// The real port a collapsed stand-in drags from (`InPort(1)`,
/// `OutPort(1)`), else the port itself.
fn drag_port_name(editor: &Editor, node: i32, port: &PortLayout) -> Option<String> {
    if !port.collapsed {
        return Some(port.name.clone());
    }
    let data = editor.document.node(node)?;
    let ports = if port.is_input { grid_inputs(data) } else { grid_outputs(data) };
    ports.get(1).map(|second| second.name.clone())
}

/// A dragged connection as it is let go: the port it was pulled from.
struct DraggedConnection {
    node: i32,
    port: String,
    from_output: bool,
    /// Pulled out of a compact node's collapsed ports.
    collapsed: bool,
}

/// What a dragged connection let go on a port does
/// (`NodeElement_OnConnectionManipulationCompleted`, `CreateConnection`).
#[derive(Clone, Debug, PartialEq)]
enum PortDrop {
    /// An output feeds an input.
    Connect { from: i32, from_port: String, to: i32, to_port: String },
    /// Output on output: a new Combine takes both (`CombineOutPorts`).
    CombineOutputs { dragged: i32, dragged_port: String, other: i32, other_port: String },
    /// On a collapsed stand-in: that node's ports to choose from
    /// (`ShowPortsMenu(ToNode, hidePrimary: true)`).
    PortsMenu { node: i32 },
    /// From collapsed ports onto another node's: both ports picked in the
    /// Create Connection window (`ShowCollapsedOutOutMenu`).
    CreateConnection { node: i32 },
    /// Input on input, or on its own node (`IsConnectionValid`).
    Nothing,
}

/// Where a connection let go on a node's `port` ends up.
fn port_drop(dragged: &DraggedConnection, node: i32, port: &str, is_input: bool) -> PortDrop {
    if node == dragged.node {
        return PortDrop::Nothing;
    }
    if port == COLLAPSED_PORT {
        if dragged.collapsed {
            return PortDrop::CreateConnection { node };
        }
        return PortDrop::PortsMenu { node };
    }
    let dragged_port = dragged.port.clone();
    let port = port.to_string();
    match (dragged.from_output, is_input) {
        (true, true) => PortDrop::Connect { from: dragged.node, from_port: dragged_port, to: node, to_port: port },
        (true, false) => PortDrop::CombineOutputs { dragged: dragged.node, dragged_port, other: node, other_port: port },
        (false, false) => PortDrop::Connect { from: node, from_port: port, to: dragged.node, to_port: dragged_port },
        (false, true) => PortDrop::Nothing,
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_port_drags(ui: &Ui, editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], ports: &[(usize, PortLayout, Response)], origin: Pos2, commands: &mut Vec<Command>) {
    let double_click_connect = editor.user.options.double_click_connect();
    for (index, port, response) in ports {
        let node = layouts[*index].id;
        let collapsed = port.collapsed;
        // A port has no menu of its own: a right-click on it goes on to
        // its node's.
        node_menu(response, editor, state, node, commands);
        // `PortElement.OnMouseDoubleClick` (Double-Click Connect): a
        // connection follows the pointer until the next click.
        if double_click_connect && response.double_clicked() {
            let Some(name) = drag_port_name(editor, node, port) else { continue };
            state.drag = if port.is_input {
                Some(Drag::FromInput { node, port: name, sticky: true, collapsed })
            } else {
                Some(Drag::FromOutput { node, port: name, sticky: true, collapsed })
            };
            continue;
        }
        if !response.drag_started_by(PointerButton::Primary) {
            continue;
        }
        let Some(name) = drag_port_name(editor, node, port) else { continue };
        if !port.is_input {
            state.drag = Some(Drag::FromOutput { node, port: name, sticky: false, collapsed });
            continue;
        }
        // Pulling a connected input detaches its connection. A connection
        // from a node the project no longer has (one of an unknown type,
        // dropped on open) is not drawn, so the input acts unconnected. The
        // collapsed stand-in is no connection's end.
        let record = editor.document.node(node).and_then(|data| data.port(&name)).and_then(|data| data.record.clone());
        let source_exists = record.as_ref().is_some_and(|record| editor.document.node(record.from).is_some());
        match record {
            Some(record) if source_exists && !collapsed => {
                editor.document.disconnect(node, &name);
                editor.changed();
                state.drag = Some(Drag::FromOutput { node: record.from, port: record.from_port, sticky: false, collapsed: false });
            }
            _ => state.drag = Some(Drag::FromInput { node, port: name, sticky: false, collapsed }),
        }
    }

    let sticky = matches!(state.drag, Some(Drag::FromOutput { sticky: true, .. } | Drag::FromInput { sticky: true, .. }));
    let ended = if sticky {
        let started_now = ports.iter().any(|(_, _, response)| response.double_clicked());
        let clicked = ui.input(|input| input.pointer.primary_clicked());
        clicked && !started_now
    } else {
        ui.input(|input| input.pointer.primary_released())
    };
    if !ended {
        return;
    }
    let Some(drag) = state.drag.clone() else {
        return;
    };
    let dragged = match drag {
        Drag::FromOutput { node, port, collapsed, .. } => DraggedConnection { node, port, from_output: true, collapsed },
        Drag::FromInput { node, port, collapsed, .. } => DraggedConnection { node, port, from_output: false, collapsed },
        _ => return,
    };
    let pointer = ui.input(|input| input.pointer.interact_pos());
    let view = state.view();
    let target = pointer.and_then(|point| snapping_port(layouts, &view, origin, state, point));
    state.drag = None;
    if let Some((target_node, target_port, target_is_input)) = target {
        let at = pointer.unwrap_or(origin);
        let dropped = port_drop(&dragged, target_node, &target_port, target_is_input);
        match dropped {
            PortDrop::Connect { from, from_port, to, to_port } => {
                let command = GraphCommand::ConnectPorts { from, from_port, to, to_port };
                commands.push(Command::Graph(command));
            }
            PortDrop::CombineOutputs { dragged: dragged_node, dragged_port, other, other_port } => {
                let command = GraphCommand::CombineOutputs { dragged: dragged_node, dragged_port, other, other_port };
                commands.push(Command::Graph(command));
            }
            PortDrop::PortsMenu { node } => {
                state.popup = Some(GraphPopup::Ports { from: dragged.node, from_port: dragged.port, from_output: dragged.from_output, node, at });
            }
            PortDrop::CreateConnection { node } => {
                state.create_connection = Some(CreateConnection::new(dragged.node, node, dragged.from_output));
            }
            PortDrop::Nothing => {}
        }
        return;
    }
    let Some(point) = pointer.filter(|point| state.rect.contains(*point)) else {
        return;
    };
    // Let go on one of its own node's ports: nothing (`IsConnectionValid`).
    let own_layout = layouts.iter().find(|layout| layout.id == dragged.node);
    let canvas_point = view.to_canvas(origin, point);
    let on_own_port = own_layout.is_some_and(|layout| layout.ports().any(|port| port.takes(canvas_point)));
    if on_own_port {
        return;
    }
    let canvas = view.to_canvas(origin, point);
    // Dropped on a collapsed group: the group's portals are offered.
    let collapsed_group = groups.iter().find(|group| group.collapsed && group.contains([canvas.x, canvas.y]));
    if let Some(group) = collapsed_group {
        if dragged.from_output {
            return;
        }
        state.popup = Some(GraphPopup::GroupPortals { group: group.id, to: dragged.node, to_port: dragged.port, at: point });
        return;
    }
    // `OnConnectionDraggedOut`: the node search, to add a node joined to
    // the dragged port.
    let source = NodeSource::Port { node: dragged.node, port: dragged.port, output: dragged.from_output };
    let search = SearchPopup::new(SearchMode::Nodes { source }, point, [canvas.x, canvas.y]);
    state.popup = Some(GraphPopup::Search(search));
}

#[allow(clippy::too_many_arguments)]
fn handle_node_interaction(ui: &Ui, editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], nodes: &[(usize, Response)], origin: Pos2, commands: &mut Vec<Command>) {
    let view = state.view();
    let modifiers = ui.input(|input| input.modifiers);
    for (index, response) in nodes {
        let id = layouts[*index].id;
        // Beside the pointer, not under the node's corner, which at a
        // high zoom is far off or past the graph's edge.
        let response = if state.popup.is_none() {
            let tooltip = node_tooltip(editor, id);
            response.clone().tooltip_at_pointer(tooltip)
        } else {
            response.clone()
        };
        if response.clicked() {
            select_clicked_node(editor, state, layouts, id, modifiers);
        }
        // `NodeElement.OnMouseDoubleClick`: `CycleNodeSizes`.
        if response.double_clicked() {
            commands.push(Command::CycleNodeSize(id));
        }
        if response.drag_started_by(PointerButton::Primary) && state.drag.is_none() {
            if modifiers.command || modifiers.shift || !editor.selection.contains(&id) {
                select_clicked_node(editor, state, layouts, id, modifiers);
            }
            let pointer = drag_origin(ui, &response).unwrap_or(origin);
            let pointer_start = view.to_canvas(origin, pointer);
            start_element_drag(editor, state, id, pointer_start);
        }
        node_menu(&response, editor, state, id, commands);
    }
    drag_elements(ui, editor, state, layouts, groups, origin);
}

fn select_clicked_node(editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], node: i32, modifiers: egui::Modifiers) {
    if modifiers.command {
        editor.toggle_selected(node);
        return;
    }
    if !modifiers.shift {
        state.clear_element_selection();
        editor.select(Some(node));
        return;
    }

    // `ShiftDragSelectElement`: retain the selection and fill the clicked
    // node's parents or children when they contain a selected node. The
    // first selected node bounds the traversal on either side.
    let mut selection = editor.selection.clone();
    if let Some(anchor) = selection.first().copied() {
        let visible: HashSet<i32> = layouts.iter().map(|layout| layout.id).collect();
        for upstream in [true, false] {
            let related = selection_neighbours(editor.document.terrain(), node, anchor, upstream, &visible);
            let connected = related.iter().any(|id| editor.selection.contains(id));
            if !connected {
                continue;
            }
            for id in related {
                if !selection.contains(&id) {
                    selection.push(id);
                }
            }
        }
    }
    if !selection.contains(&node) {
        selection.push(node);
    }
    editor.set_selection(selection);
}

fn selection_neighbours(terrain: &nerthus_engine::Terrain, start: i32, anchor: i32, upstream: bool, visible: &HashSet<i32>) -> Vec<i32> {
    let mut adjacency: HashMap<i32, Vec<i32>> = HashMap::new();
    for node in terrain.nodes.values() {
        if !visible.contains(&node.id) {
            continue;
        }
        for port in node.ports.iter().filter(|port| port.is_input()) {
            let Some(record) = &port.record else {
                continue;
            };
            if !visible.contains(&record.from) {
                continue;
            }
            let (from, to) = if upstream { (node.id, record.from) } else { (record.from, node.id) };
            adjacency.entry(from).or_default().push(to);
        }
    }

    let mut related = Vec::new();
    let mut pending = vec![start];
    let mut seen = HashSet::from([start]);
    while let Some(current) = pending.pop() {
        let Some(neighbours) = adjacency.get(&current) else {
            continue;
        };
        for neighbour in neighbours {
            if !seen.insert(*neighbour) {
                continue;
            }
            related.push(*neighbour);
            if *neighbour != anchor {
                pending.push(*neighbour);
            }
        }
    }
    related
}

/// The node's context menu on a right-click of the node or one of its
/// ports, the node selected unless it already is.
fn node_menu(response: &Response, editor: &mut Editor, state: &mut GraphState, id: i32, commands: &mut Vec<Command>) {
    crate::ui::widgets::context_menu(response, |ui| {
        state.clear_element_selection();
        if !editor.selection.contains(&id) {
            editor.select(Some(id));
        }
        node_context_menu(ui, editor, commands);
    });
}

/// The drag of the selection begins: locked elements stay (`Locked`), and
/// a group's nodes move with the group rather than on their own.
fn start_element_drag(editor: &mut Editor, state: &mut GraphState, primary: i32, pointer_start: Pos2) {
    let groups: Vec<(i32, [f32; 2])> = state.selected_groups.iter().filter_map(|id| editor.document.group(*id)).filter(|group| !group.locked).map(|group| (group.id, group.position)).collect();
    let carried: HashSet<i32> = groups.iter().filter_map(|(id, _)| editor.document.group(*id)).flat_map(|group| group.children).collect();
    let mut nodes = Vec::new();
    for selected in &editor.selection {
        if carried.contains(selected) || editor.document.node_locked_in_group(*selected) {
            continue;
        }
        if let Some(node) = editor.document.node(*selected) {
            nodes.push((*selected, node.position));
        }
    }
    let note = state.selected_note.filter(|note| !carried.contains(note)).and_then(|note| note_corner(editor, note).map(|corner| (note, corner)));
    editor.document.checkpoint("Move");
    state.drag = Some(Drag::Elements { pointer_start, primary, nodes, note, groups });
}

fn note_corner(editor: &Editor, id: i32) -> Option<[f32; 2]> {
    let notes = editor.document.notes();
    let fields = notes.iter().find(|(note, _)| *note == id).map(|(_, fields)| *fields)?;
    let position = fields.get("Position")?;
    let x = position.get("X").and_then(Value::as_f64)? as f32;
    let y = position.get("Y").and_then(Value::as_f64)? as f32;
    Some([x, y])
}

fn drag_elements(ui: &Ui, editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], origin: Pos2) {
    let Some(Drag::Elements { pointer_start, primary, nodes, note, groups: dragged_groups }) = state.drag.clone() else {
        return;
    };
    let view = state.view();
    let escaped = ui.input(|input| input.key_pressed(Key::Escape)) && !ui.ctx().text_edit_focused();
    let dragging = ui.input(|input| input.pointer.primary_down()) && !escaped;
    if let Some(pointer) = ui.input(|input| input.pointer.interact_pos()) {
        let current = view.to_canvas(origin, pointer);
        let delta = current - pointer_start;
        let place = |start: [f32; 2]| {
            let mut x = start[0] + delta.x;
            let mut y = start[1] + delta.y;
            if state.snap_to_grid {
                x = snap_to_unit(x, state.grid_unit);
                y = snap_to_unit(y, state.grid_unit);
            }
            [x, y]
        };
        let mut moves: Vec<(i32, [f32; 2])> = nodes.iter().map(|(id, start)| (*id, place(*start))).collect();
        // Snap to Items aligns the node grabbed; the others keep their
        // places beside it.
        state.snap_lines.clear();
        let primary_move = moves.iter().find(|(id, _)| *id == primary).map(|(_, position)| *position);
        if editor.user.options.snap_to_items()
            && let Some(position) = primary_move
            && let Some(layout) = layouts.iter().find(|layout| layout.id == primary)
        {
            let size = layout.rect.size();
            let moving: HashSet<i32> = nodes.iter().map(|(id, _)| *id).collect();
            let visible = Rect::from_min_size(view.location.to_pos2(), state.rect.size() / view.zoom);
            let others: Vec<Rect> = layouts.iter().filter(|other| !moving.contains(&other.id) && visible.contains(other.rect.min)).map(|other| other.rect).collect();
            let (offset, lines) = snap_to_items(Rect::from_min_size(Pos2::new(position[0], position[1]), size), &others);
            for (_, moved) in moves.iter_mut() {
                moved[0] += offset.x;
                moved[1] += offset.y;
            }
            state.snap_lines = lines;
        }
        editor.document.move_nodes(&moves, false);
        if let Some((id, start)) = note {
            let moved = place(start);
            editor.document.set_note_vector(id, "Position", moved, false);
        }
        for (id, start) in &dragged_groups {
            let moved = place(*start);
            editor.document.move_group(*id, moved, false);
        }
        update_group_states(editor, state, groups, primary, current);
    }
    if dragging {
        return;
    }
    // `NodeElement_MoveCompleted`: what was dragged out of its group's box
    // leaves it; what was dragged into one joins it.
    let tab = state.tab;
    let mut settled: Vec<(i32, [f32; 2], usize)> = Vec::new();
    for (id, _) in &nodes {
        if let Some(node) = editor.document.node(*id) {
            settled.push((*id, node.position, tab));
        }
    }
    if let Some((id, _)) = note
        && let Some(corner) = note_corner(editor, id)
    {
        settled.push((id, corner, tab));
    }
    editor.document.settle_in_groups(&settled);
    state.drag = None;
    state.snap_lines.clear();
    state.group_states.clear();
}

/// While a node is dragged: the group under the pointer that would take
/// it shows `Incoming`, the group it would leave `Outgoing`.
fn update_group_states(editor: &Editor, state: &mut GraphState, groups: &[GroupData], primary: i32, pointer: Pos2) {
    state.group_states.clear();
    let owner = editor.document.group_of(primary);
    for group in groups {
        let over = group.contains([pointer.x, pointer.y]);
        let owns = owner == Some(group.id);
        if over && !owns {
            state.group_states.insert(group.id, GroupState::Incoming);
        } else if !over && owns {
            state.group_states.insert(group.id, GroupState::Outgoing);
        }
    }
}

/// `PrepareToPushElements`: every node, group and note of the tab whose
/// corner lies right of `x`, with where it is. Nodes and notes of a group
/// that is pushed go with it, not on their own.
fn elements_to_push(document: &Document, tab: usize, x: f32) -> PushedElements {
    let mut elements = PushedElements::default();
    let mut carried: HashSet<i32> = HashSet::new();
    for group in document.groups() {
        if group.graph_index != tab || group.position[0] <= x {
            continue;
        }
        carried.extend(group.children.iter().copied());
        elements.groups.push((group.id, group.position));
    }
    for node in document.terrain().nodes.values() {
        if node_tab(node) != tab || node.position[0] <= x || carried.contains(&node.id) {
            continue;
        }
        elements.nodes.push((node.id, node.position));
    }
    for (id, fields) in document.notes() {
        let graph_index = fields.get("GraphIndex").and_then(Value::as_u64).unwrap_or(0) as usize;
        let position = fields.get("Position");
        let coordinate = |axis: &str| position.and_then(|position| position.get(axis)).and_then(Value::as_f64).unwrap_or(0.0) as f32;
        let corner = [coordinate("X"), coordinate("Y")];
        if graph_index != tab || corner[0] <= x || carried.contains(&id) {
            continue;
        }
        elements.notes.push((id, corner));
    }
    elements
}

/// A push begins at `press` (canvas units): one undo step for it, when
/// there is anything to push.
fn start_push(editor: &mut Editor, state: &mut GraphState, press: Pos2) {
    let elements = elements_to_push(&editor.document, state.tab, press.x);
    if !elements.is_empty() {
        editor.document.checkpoint("Push");
    }
    state.drag = Some(Drag::Push { press, elements });
}

/// `PushElements`: what lay right of where the push began moves across
/// with the pointer (back too), until the button comes up.
fn push_elements(ui: &Ui, editor: &mut Editor, state: &mut GraphState, origin: Pos2) {
    let Some(Drag::Push { press, elements }) = state.drag.clone() else {
        return;
    };
    let view = state.view();
    let pointer = ui.input(|input| input.pointer.interact_pos());
    if let Some(pointer) = pointer {
        let shift = view.to_canvas(origin, pointer).x - press.x;
        let pushed = |start: [f32; 2]| [start[0] + shift, start[1]];
        for (id, start) in &elements.groups {
            editor.document.move_group(*id, pushed(*start), false);
        }
        let moves: Vec<(i32, [f32; 2])> = elements.nodes.iter().map(|(id, start)| (*id, pushed(*start))).collect();
        editor.document.move_nodes(&moves, false);
        for (id, start) in &elements.notes {
            editor.document.set_note_vector(*id, "Position", pushed(*start), false);
        }
    }
    ui.ctx().set_cursor_icon(CursorIcon::ResizeEast);
    if !ui.input(|input| input.pointer.primary_down()) {
        state.drag = None;
    }
}

/// `VerticalDragRect`: from where the push began to the pointer, the
/// graph's whole height, behind everything else.
fn draw_push_band(painter: &egui::Painter, rect: Rect, view: &GraphView, origin: Pos2, press: Pos2, pointer: Option<Pos2>) {
    let Some(pointer) = pointer else {
        return;
    };
    let left = view.to_screen(origin, press).x;
    if pointer.x <= left {
        return;
    }
    // Past the graph's top and bottom, so only the sides' lines show.
    let top_left = Pos2::new(left, rect.top() - 2.0);
    let bottom_right = Pos2::new(pointer.x, rect.bottom() + 2.0);
    let band = Rect::from_min_max(top_left, bottom_right);
    // The rectangle's `Opacity` 0.75 over its fill and line.
    let fill = theme::ACCENT_TRANSPARENT_PURPLE.gamma_multiply(0.75);
    let line = Stroke::new(view.zoom, theme::ACCENT_PURPLE.gamma_multiply(0.75));
    painter.rect_filled(band, 0.0, fill);
    painter.rect_stroke(band, 0.0, line, StrokeKind::Inside);
}

/// `SnapToOtherShapes`: the dragged box's edges and centres against each
/// other node's, closest first within 36; returns the shift and the lines
/// that show it.
fn snap_to_items(dragged: Rect, others: &[Rect]) -> (Vec2, Vec<[Pos2; 2]>) {
    let mut best_x: Option<(f32, f32, f32, Rect)> = None;
    let mut best_y: Option<(f32, f32, f32, Rect)> = None;
    let consider = |best: &mut Option<(f32, f32, f32, Rect)>, distance: f32, shift: f32, line_at: f32, other: Rect| {
        let closest = best.map_or(SNAP_ITEMS_DISTANCE, |(found, _, _, _)| found);
        if distance < closest {
            *best = Some((distance, shift, line_at, other));
        }
    };
    for other in others {
        // Top to top, bottom to bottom.
        consider(&mut best_y, (dragged.top() - other.top()).abs(), other.top() - dragged.top(), other.top(), *other);
        consider(&mut best_y, (dragged.bottom() - other.bottom()).abs(), other.bottom() - dragged.bottom(), other.bottom(), *other);
        // Right against left, left against right, then the centres.
        consider(&mut best_x, (dragged.right() - other.left()).abs(), other.left() - dragged.right(), other.left(), *other);
        consider(&mut best_x, (dragged.left() - other.right()).abs(), other.right() - dragged.left(), other.right(), *other);
        consider(&mut best_x, (dragged.center().x - other.center().x).abs(), other.center().x - dragged.center().x, other.center().x, *other);
        consider(&mut best_y, (dragged.center().y - other.center().y).abs(), other.center().y - dragged.center().y, other.center().y, *other);
    }
    let shift_x = best_x.map_or(0.0, |(_, shift, _, _)| shift);
    let shift_y = best_y.map_or(0.0, |(_, shift, _, _)| shift);
    let placed = dragged.translate(Vec2::new(shift_x, shift_y));
    let mut lines = Vec::new();
    if let Some((_, _, x, other)) = best_x {
        let top = placed.top().min(other.top());
        let bottom = placed.bottom().max(other.bottom());
        lines.push([Pos2::new(x, top), Pos2::new(x, bottom)]);
    }
    if let Some((_, _, y, other)) = best_y {
        let left = placed.left().min(other.left());
        let right = placed.right().max(other.right());
        lines.push([Pos2::new(left, y), Pos2::new(right, y)]);
    }
    (Vec2::new(shift_x, shift_y), lines)
}

/// The node's right-click menu (`_ctxNodeMenu` as `PrepareNodeMenu`
/// builds it). With several nodes selected only what applies to all shows
/// (`RefreshNodeMenu`), and check marks stand for the selection
/// (`_ctxNodeMenu_Opened`).
pub fn node_context_menu(ui: &mut Ui, editor: &Editor, commands: &mut Vec<Command>) {
    menu_style(ui);
    let several = editor.selection.len() > 1;
    let active = editor.active_node();
    let node = active.and_then(|id| editor.document.node(id));
    let selected_nodes: Vec<&NodeData> = editor.selection.iter().filter_map(|id| editor.document.node(*id)).collect();
    let bypassed = |node: &NodeData| node.base.get("StateOverride").and_then(Value::as_str) == Some("Bypassed");

    if menu_item(ui, None, None, "Refresh", "F5").clicked() {
        commands.push(Command::Refresh);
    }
    if !several {
        if menu_item(ui, Some("rotate-360"), Some(theme::ACCENT_BLUE), "Reset", "Ctrl R").clicked() {
            commands.push(Command::ResetNode);
        }
        if menu_item(ui, None, None, "Rename", "F2").clicked() {
            commands.push(Command::Rename);
        }
    }
    if menu_item(ui, Some("stack"), None, "Duplicate", "Ctrl D").clicked() {
        commands.push(Command::Duplicate);
    }
    menu_separator(ui);

    let all_bypassed = !selected_nodes.is_empty() && selected_nodes.iter().all(|node| bypassed(node));
    if checked_icon_item(ui, all_bypassed, Some("road-sign"), None, "Bypass", "B") {
        commands.push(Command::Bypass);
    }
    if !several {
        let id = active.unwrap_or_default();
        let bookmarked = editor.document.bookmarks().contains(&id);
        if checked_icon_item(ui, bookmarked, None, None, "Bookmark", "") {
            commands.push(Command::ToggleBookmark);
        }
        let locked = editor.preview_locked && editor.preview_node == active;
        if checked_icon_item(ui, locked, None, None, "Lock Preview", "F") {
            commands.push(Command::LockPreview);
        }
        let underlay = active.is_some() && editor.document.underlay_node() == id;
        if checked_icon_item(ui, underlay, None, None, "Use as Underlay", "G") {
            commands.push(Command::ToggleUnderlay);
        }
        let ignored = node.and_then(|node| node.base.get("IgnoreUnderlay")).and_then(Value::as_bool).unwrap_or(false);
        if checked_icon_item(ui, ignored, None, None, "Exclude from Underlay", "") {
            commands.push(Command::ToggleIgnoreUnderlay);
        }
    }
    // One node: exporting and enabled; several: all marked.
    let exporting = if several {
        selected_nodes.iter().all(|node| ExportSettings::of(node).is_some())
    } else {
        node.and_then(ExportSettings::of).is_some_and(|settings| settings.enabled)
    };
    if checked_icon_item(ui, exporting, Some("device-floppy"), Some(theme::ACCENT_BLUE), "Mark for Export", "F3") {
        commands.push(Command::MarkForExport);
    }
    menu_separator(ui);

    if !several {
        if menu_item(ui, Some("replace"), Some(theme::ACCENT_MUTED_GREEN), "Replace", "Ctrl Q").clicked() {
            commands.push(Command::Graph(GraphCommand::ReplaceSearch));
        }
        if menu_item(ui, None, None, "Insert chokepoint", "").clicked()
            && let Some(id) = active
        {
            commands.push(Command::Graph(GraphCommand::ChokepointMenu(id)));
        }
        if menu_item(ui, None, None, "Manage Portals", "P").clicked() {
            commands.push(Command::Graph(GraphCommand::ManagePortals));
        }
    } else {
        submenu(ui, None, None, "Combine into", |ui| {
            let entries = [
                ("x-combine", "Combine as Inputs", "F8", crate::document::CombineKind::Inputs),
                ("x-mixer", "Mixer as Color Layers", "Ctrl F8", crate::document::CombineKind::Mixer),
                ("x-layers", "Layers as Normal Layers", "Ctrl Shift F8", crate::document::CombineKind::Layers),
            ];
            for (icon, text, shortcut, kind) in entries {
                if menu_item(ui, Some(icon), None, text, shortcut).clicked() {
                    commands.push(Command::Graph(GraphCommand::Combine(kind)));
                }
            }
        });
    }
    if menu_item(ui, Some("square-rounded-x"), Some(theme::ACCENT_RED), "Delete", "Del").clicked() {
        commands.push(Command::Delete);
    }
    if several {
        return;
    }
    menu_separator(ui);
    if menu_item(ui, None, None, "Copy Properties", "").clicked() {
        commands.push(Command::CopyProperties);
    }
    // Only onto a node of the copied node's type.
    let copied_type = editor.copied_node.as_ref().map(|copied| copied.type_name.as_str());
    let can_paste = node.is_some_and(|node| copied_type == Some(node.type_name.as_str()));
    if menu_item_enabled(ui, None, None, "Paste Properties", "", can_paste).clicked() {
        commands.push(Command::PasteProperties);
    }
    menu_separator(ui);
    if menu_item(ui, Some("picture-in-picture"), Some(theme::ACCENT_MUTED_BLUE), "Show Locked 2D Viewport", "").clicked() {
        commands.push(Command::ShowLocked2DViewport);
    }
}

#[allow(clippy::too_many_arguments)]
fn handle_background(
    ui: &Ui,
    editor: &mut Editor,
    state: &mut GraphState,
    layouts: &[NodeLayout],
    groups: &[GroupData],
    background: &Response,
    connections: &[ConnectionLayout],
    hovered_connection: Option<usize>,
    origin: Pos2,
    commands: &mut Vec<Command>,
) {
    let view = state.view();
    let modifiers = ui.input(|input| input.modifiers);
    let additive = modifiers.command || modifiers.shift;
    // Ctrl and Shift together push elements aside instead of selecting.
    let pushing = modifiers.command && modifiers.shift;
    let hovered = hovered_connection.map(|index| &connections[index]);
    if background.clicked() {
        match hovered {
            Some(connection) => {
                let key = (connection.to, connection.to_port.clone());
                if additive {
                    toggle_connection(state, key);
                } else {
                    state.clear_element_selection();
                    editor.select(None);
                    state.selected_connections = vec![key];
                }
            }
            None if pushing => {}
            None => {
                state.clear_element_selection();
                editor.select(None);
            }
        }
    }
    // A connection right-clicked: its menu (`_ctxConnection`).
    if background.secondary_clicked()
        && let Some(point) = background.interact_pointer_pos()
    {
        state.context_point = Some(view.to_canvas(origin, point));
        if let Some(connection) = hovered {
            let key = (connection.to, connection.to_port.clone());
            if !state.selected_connections.contains(&key) {
                state.clear_element_selection();
                editor.select(None);
                state.selected_connections = vec![key.clone()];
            }
            state.popup = Some(GraphPopup::Connection { to: key.0, to_port: key.1, at: point });
        }
    }
    if background.drag_started_by(PointerButton::Primary) && state.drag.is_none() {
        // The push and the box both reach back to where the button went down.
        let press = drag_origin(ui, background).unwrap_or(origin);
        if pushing {
            let canvas = view.to_canvas(origin, press);
            start_push(editor, state, canvas);
        } else {
            state.drag = Some(Drag::Marquee { start: press, additive });
        }
    }
    push_elements(ui, editor, state, origin);
    if let Some(Drag::Marquee { start, additive }) = state.drag.clone() {
        let released = !ui.input(|input| input.pointer.primary_down());
        if released {
            let end = ui.input(|input| input.pointer.interact_pos()).unwrap_or(start);
            let screen_box = Rect::from_two_pos(start, end);
            select_in_box(editor, state, layouts, groups, connections, screen_box, additive);
            state.drag = None;
        }
    }
    // The graph's menu opens right-clicked on empty graph; right-clicking a
    // connection opens that connection's menu above instead, so this one is
    // not opened or moved over it. Egui drops a menu that is not shown every
    // frame as abandoned, so an open one is re-shown even when the pointer
    // (on the menu or off it) happens to cross a connection.
    let on_connection = hovered.is_some();
    let right_click_on_connection = on_connection && background.secondary_clicked();
    if (background.context_menu_opened() || !on_connection) && !right_click_on_connection {
        crate::ui::widgets::context_menu(background, |ui| {
            canvas_context_menu(ui, editor, state, commands);
        });
    }
    // Keys while the pointer is over the graph and nothing types.
    let typing = ui.ctx().text_edit_focused();
    let over = ui.input(|input| input.pointer.hover_pos()).is_some_and(|point| state.rect.contains(point));
    if !typing && over && ui.input(|input| input.key_pressed(Key::Escape)) {
        state.drag = None;
        state.snap_lines.clear();
        state.group_states.clear();
        state.selected_connections.clear();
    }
}

fn toggle_connection(state: &mut GraphState, key: (i32, String)) {
    if let Some(position) = state.selected_connections.iter().position(|selected| *selected == key) {
        state.selected_connections.remove(position);
    } else {
        state.selected_connections.push(key);
    }
}

/// `SelectItemsInRect`: nodes and notes the box touches, groups wholly in
/// it, connections crossing it; locked ones stay out.
#[allow(clippy::too_many_arguments)]
fn select_in_box(editor: &mut Editor, state: &mut GraphState, layouts: &[NodeLayout], groups: &[GroupData], connections: &[ConnectionLayout], screen_box: Rect, additive: bool) {
    let view = state.view();
    let origin = state.rect.min;
    let canvas_box = Rect::from_two_pos(view.to_canvas(origin, screen_box.min), view.to_canvas(origin, screen_box.max));
    let mut chosen: Vec<i32> = if additive { editor.selection.clone() } else { Vec::new() };
    for layout in layouts {
        let touched = layout.rect.intersects(canvas_box);
        if touched && !chosen.contains(&layout.id) && !editor.document.node_locked_in_group(layout.id) {
            chosen.push(layout.id);
        }
    }
    if !additive {
        state.clear_element_selection();
    }
    for group in groups.iter().filter(|group| !group.locked) {
        let inside = canvas_box.contains_rect(group.bounds());
        if inside && !state.selected_groups.contains(&group.id) {
            state.selected_groups.push(group.id);
        }
    }
    for connection in connections {
        let (Some(start), Some(end)) = (connection.points.first(), connection.points.last()) else { continue };
        let key = (connection.to, connection.to_port.clone());
        if segment_meets_rect(*start, *end, screen_box) && !state.selected_connections.contains(&key) {
            state.selected_connections.push(key);
        }
    }
    let tab = state.tab;
    if state.selected_note.is_none() {
        let notes = editor.document.notes();
        let touched_note = notes.iter().map(|(id, _)| *id).find(|id| crate::ui::graph_notes::note_rect(editor, tab, *id).is_some_and(|rect| rect.intersects(canvas_box)));
        state.selected_note = touched_note;
    }
    editor.set_selection(chosen);
}

/// Right-click on empty graph: add nodes by category and family.
fn canvas_context_menu(ui: &mut Ui, editor: &Editor, state: &mut GraphState, commands: &mut Vec<Command>) {
    menu_style(ui);
    crate::ui::toolbox::add_node_menus(ui, editor, state, commands);
    menu_separator(ui);
    if menu_item(ui, Some("note"), Some(theme::ACCENT_STRONG_YELLOW), "Add Note", "").clicked() {
        commands.push(Command::Graph(GraphCommand::AddNoteAt(state.context_position())));
    }
    if menu_item(ui, Some("zoom-scan"), None, "Zoom to Extents", "Ctrl 0").clicked() {
        commands.push(Command::ZoomExtents);
    }
}

/// Files dropped on the graph (`DropFiles`), where they land, and a
/// toolbox entry let go over it (`canvas_Drop`, onto a node to follow it).
fn handle_drops(ui: &Ui, editor: &Editor, state: &mut GraphState, layouts: &[NodeLayout], commands: &mut Vec<Command>) {
    let view = state.view();
    let origin = state.rect.min;
    let latest = ui.input(|input| input.pointer.latest_pos());
    let dropped: Vec<PathBuf> = ui.input(|input| input.raw.dropped_files.iter().map(|file| file.path().to_path_buf()).collect());
    // Files dropped elsewhere (a file field, say) are not the graph's; where
    // the system can't say where they landed, they go to its middle.
    let landing = match latest {
        Some(point) if ui.rect_contains_pointer(state.rect) => Some(point),
        Some(_) => None,
        None => Some(state.rect.center()),
    };
    if let (false, Some(point)) = (dropped.is_empty(), landing) {
        let canvas = view.to_canvas(origin, point);
        commands.push(Command::Graph(GraphCommand::DropFiles { files: dropped, at: [canvas.x, canvas.y] }));
    }
    if state.toolbox_drag.is_none() {
        return;
    }
    let released = ui.input(|input| input.pointer.primary_released() || !input.pointer.primary_down());
    if !released {
        return;
    }
    let Some(type_name) = state.toolbox_drag.take() else {
        return;
    };
    let Some(point) = latest.filter(|point| state.rect.contains(*point) && !state.toolbox_rect.contains(*point) && ui.rect_contains_pointer(state.rect)) else {
        return;
    };
    let canvas = view.to_canvas(origin, point);
    let onto = layouts.iter().rev().find(|layout| layout.rect.contains(canvas)).map(|layout| layout.id);
    let _ = editor;
    commands.push(Command::Graph(GraphCommand::ToolboxDrop { type_name, at: [canvas.x, canvas.y], onto }));
}

fn draw_drag_overlay(ui: &Ui, painter: &egui::Painter, state: &GraphState, layouts: &[NodeLayout], origin: Pos2, pointer: Option<Pos2>) {
    let view = state.view();
    for line in &state.snap_lines {
        let start = view.to_screen(origin, line[0]);
        let end = view.to_screen(origin, line[1]);
        painter.line_segment([start, end], Stroke::new(1.0, SNAP_LINE));
    }
    if let (Some(type_name), Some(point)) = (&state.toolbox_drag, pointer) {
        let label = Catalogue::get().node(type_name).map(|spec| spec.display_name.clone().unwrap_or_else(|| spec.name.clone())).unwrap_or_else(|| type_name.clone());
        let layer = egui::LayerId::new(egui::Order::Tooltip, ui.id().with("toolbox-drag-tag"));
        let overlay = ui.ctx().layer_painter(layer);
        let painter = &overlay;
        let galley = painter.layout_no_wrap(label, theme::semibold(11.0), theme::LABEL_EMPHASIS);
        let tag = Rect::from_min_size(point + Vec2::new(12.0, 8.0), galley.size() + Vec2::new(12.0, 6.0));
        painter.rect_filled(tag, 3.0, theme::MENU_BACKGROUND);
        painter.rect_stroke(tag, 3.0, Stroke::new(1.0, theme::MENU_BORDER), StrokeKind::Inside);
        painter.galley(tag.min + Vec2::new(6.0, 3.0), galley, theme::LABEL_EMPHASIS);
    }
    match &state.drag {
        Some(Drag::Marquee { start, .. }) => {
            let end = pointer.unwrap_or(*start);
            let marquee = Rect::from_two_pos(*start, end);
            painter.rect_filled(marquee, 0.0, theme::SELECTION_FILL);
            painter.rect_stroke(marquee, 0.0, Stroke::new(1.0, theme::SELECTION_STROKE), StrokeKind::Inside);
        }
        Some(Drag::FromOutput { node, port, .. }) | Some(Drag::FromInput { node, port, .. }) => {
            let is_input = matches!(state.drag, Some(Drag::FromInput { .. }));
            let Some(layout) = layouts.iter().find(|layout| layout.id == *node) else { return };
            let Some(port_layout) = layout.port(port, is_input) else { return };
            let anchor = view.to_screen(origin, port_layout.anchor);
            let Some(end) = pointer else { return };
            // `ConnectionLine`: a dashed gradient line that crawls.
            let time = ui.input(|input| input.time) as f32;
            let offset = (time * 16.0) % 5.0;
            let points = [anchor, end];
            let mid = theme::CONNECTION_LINE_START.lerp_to_gamma(theme::CONNECTION_LINE_END, 0.5);
            let stroke = Stroke::new(1.5, mid);
            let dashes = Shape::dashed_line_with_offset(&points, stroke, &[2.0 * 1.5], &[3.0 * 1.5], offset);
            painter.extend(dashes);
            ui.ctx().request_repaint();
        }
        _ => {}
    }
}

/// The graph's one rename popup (`editablePopup`), for nodes and groups.
fn rename_popup_id() -> egui::Id {
    egui::Id::new("graph-rename")
}

/// Closes the rename popup (F2) without taking its text.
pub(crate) fn close_rename_popup(ctx: &egui::Context, state: &mut GraphState) {
    slider_popups::close_typing(ctx, rename_popup_id());
    state.stop_renaming();
}

/// The rename popup (F2, an `EditablePopup`) on an element at `screen`,
/// `width` wide unzoomed: a text box at 100% (`RelativePoint`, 10 in
/// from the element's top-left in the element's own units, so zoomed with
/// it; as wide as the element is unzoomed, 96 at least), over everything
/// and kept in the window as a popup is. Returns the text it closed
/// taking ([`slider_popups::typing_box`] says when); the graph forgets
/// the rename once it has closed.
pub(crate) fn rename_popup(ui: &Ui, state: &mut GraphState, screen: Rect, width: f32, zoom: f32) -> Option<String> {
    let id = rename_popup_id();
    if let Some(text) = state.rename_start.take() {
        slider_popups::open_typing(ui.ctx(), id, text);
    }
    let popup_scale = slider_popups::popup_scale(ui.ctx());
    let look = slider_popups::TypingLook {
        anchor: screen,
        top_left: screen.min + Vec2::splat(RENAME_OFFSET * zoom),
        width: slider_popups::BoxWidth::Fixed(width.max(RENAME_MIN_WIDTH) * popup_scale),
        font: theme::interface(RENAME_FONT_SIZE),
    };
    let typed = slider_popups::typing_box(ui, id, &look);
    if !slider_popups::typing_open(ui.ctx(), id) {
        state.renaming = None;
    }
    typed
}

/// `RenameNode`: the rename popup on the node it is open on; a blank name
/// is no name. The name taken goes to [`GraphCommand::RenameNode`].
fn rename_overlay(ui: &mut Ui, state: &mut GraphState, layouts: &[NodeLayout], origin: Pos2, commands: &mut Vec<Command>) {
    let id = match state.renaming {
        Some(Renamed::Node(id)) => id,
        Some(Renamed::Group(_)) => return,
        None => {
            // Closed by the graph (a reset, another tab): the popup goes
            // with it.
            slider_popups::close_typing(ui.ctx(), rename_popup_id());
            return;
        }
    };
    let Some(layout) = layouts.iter().find(|layout| layout.id == id) else {
        close_rename_popup(ui.ctx(), state);
        return;
    };
    let view = state.view();
    let screen = view.rect_to_screen(origin, layout.rect);
    let Some(text) = rename_popup(ui, state, screen, layout.rect.width(), view.zoom) else {
        return;
    };
    let name = text.trim().to_string();
    if !name.is_empty() {
        let rename = GraphCommand::RenameNode { node: id, name };
        commands.push(Command::Graph(rename));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_toolbox_drag_tag_paints_above_later_canvas_widgets() {
        let context = themed_context();
        let mut state = GraphState::default();
        state.toolbox_drag = Some("Plates".to_string());
        let point = Pos2::new(50.0, 60.0);
        let output = context.run_ui(egui::RawInput::default(), |ui| {
            let painter = ui.painter().clone();
            draw_drag_overlay(ui, &painter, &state, &[], Pos2::ZERO, Some(point));
            let toolbox = Rect::from_min_size(Pos2::ZERO, Vec2::splat(200.0));
            painter.rect_filled(toolbox, 0.0, Color32::RED);
        });
        let label = output.shapes.iter().position(|clipped| matches!(&clipped.shape, Shape::Text(text) if text.galley.text() == "Plates")).unwrap();
        let toolbox = output.shapes.iter().position(|clipped| matches!(&clipped.shape, Shape::Rect(rect) if rect.fill == Color32::RED)).unwrap();
        assert!(label > toolbox);
        output.drop_without_applying_deltas();
    }

    #[test]
    fn a_small_locked_export_nodes_build_time_stays_inside_its_border() {
        let context = themed_context();
        let mut editor = Editor::new(Document::new());
        let node = editor.document.add_node("Combine", [0.0, 0.0]).unwrap();
        editor.document.apply_build_update(node, "NodeSize", serde_json::json!("Small"));
        editor.document.apply_build_update(node, "SaveDefinition", serde_json::json!({}));
        editor.preview_node = Some(node);
        editor.preview_locked = true;
        let status = crate::editor::NodeStatus { milliseconds: 123_450.0, ..Default::default() };
        editor.statuses.insert(node, status);
        let layout = layout_node(editor.document.node(node).unwrap(), "Standard");
        for zoom in [0.5, 1.0, 2.0] {
            let screen = Rect::from_min_size(Pos2::new(100.0, 100.0), layout.rect.size() * zoom);
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                draw_node(ui, ui.painter(), &editor, &layout, screen, zoom);
            });
            for clipped in &output.shapes {
                if let Shape::Text(text) = &clipped.shape
                    && text.override_text_color == Some(BUILD_TIME_COLOR)
                {
                    let text_rect = Rect::from_min_size(text.pos, text.galley.size());
                    assert!(screen.shrink(zoom).contains_rect(text_rect));
                }
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn grids_at_extreme_coordinates_have_a_bounded_number_of_lines() {
        let context = themed_context();
        for coordinate in [268_435_456.0, -1_073_741_824.0, f32::MAX, f32::NAN] {
            let view = GraphView { location: Vec2::splat(coordinate), zoom: 1.0 };
            let output = context.run_ui(egui::RawInput::default(), |ui| {
                draw_grid(ui.painter(), SCREEN, &view);
            });
            assert!(output.shapes.len() < 500, "coordinate {coordinate}");
            for clipped in &output.shapes {
                assert!(clipped.shape.visual_bounding_rect().is_finite());
            }
            output.drop_without_applying_deltas();
        }
    }

    #[test]
    fn a_reused_tab_starts_with_a_fresh_view_and_loading_keeps_the_requested_tab() {
        let mut editor = Editor::new(Document::new());
        let tab = editor.document.add_graph_tab();
        let node = editor.document.place_node("Mountain", [26100.0, 26100.0], tab).unwrap();
        editor.document.note_selected_tab(tab);
        editor.select(Some(node));
        let mut state = GraphState::default();
        state.tab = editor.document.opening_tab();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        assert_eq!(state.tab, tab);
        assert_eq!(editor.selection, vec![node]);
        state.view_mut().location = Vec2::splat(1234.0);
        state.view_mut().zoom = 0.2;
        state.reset_tab_view(tab);
        assert_eq!(state.view().zoom, 1.0);
        assert_eq!(state.view().location, Vec2::splat(26000.0));
        state.zoom_request = Some(ZoomRequest::Center(node));
        state.reset();
        assert_eq!(state.zoom_request, None);
    }

    #[test]
    fn non_menu_additions_ignore_the_last_canvas_menu_position() {
        let mut state = GraphState::default();
        state.context_point = Some(Pos2::new(-10000.0, -20000.0));
        assert_eq!(state.context_position(), [-10000.0, -20000.0]);
        assert_eq!(state.drop_position(), [26085.0, 26160.0]);
        state.reset();
        assert_eq!(state.context_point, None);
    }

    #[test]
    fn toolbox_drops_over_the_toolbox_are_cancelled() {
        let context = themed_context();
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        state.toolbox_rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(150.0, 500.0));
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        for (point, accepted) in [(Pos2::new(100.0, 200.0), false), (Pos2::new(400.0, 200.0), true)] {
            graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(point)], egui::Modifiers::NONE);
            state.toolbox_drag = Some("Mountain".to_string());
            let commands = graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
            assert_eq!(commands.len(), usize::from(accepted));
            assert!(state.toolbox_drag.is_none());
        }
    }

    #[test]
    fn a_covered_port_cannot_take_a_click_or_snap_a_connection() {
        let context = themed_context();
        let mut editor = Editor::new(Document::new());
        let bottom = editor.document.add_node("Blur", [26100.0, 26100.0]).unwrap();
        let top = editor.document.add_node("Mountain", [26090.0, 26120.0]).unwrap();
        let source = editor.document.add_node("Mountain", [26600.0, 26400.0]).unwrap();
        let mut state = GraphState::default();
        let none = egui::Modifiers::NONE;
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        let point = port_dot(&editor, &state, bottom, "In", true);
        let button = |pressed| egui::Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers: none };
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(point)], none);
        graph_frame(&context, &mut editor, &mut state, vec![button(true)], none);
        graph_frame(&context, &mut editor, &mut state, vec![button(false)], none);
        assert_eq!(editor.selection, vec![top]);
        assert!(state.drag.is_none());
        let layouts: Vec<NodeLayout> = [bottom, top, source].iter().map(|id| layout_node(editor.document.node(*id).unwrap(), "Standard")).collect();
        state.drag = Some(Drag::FromOutput { node: source, port: "Out".to_string(), sticky: false, collapsed: false });
        assert!(snapping_port(&layouts, &state.view(), state.rect.min, &state, point).is_none());
    }

    #[test]
    fn escape_finishes_a_node_drop_and_clears_drag_feedback() {
        let context = themed_context();
        let mut editor = Editor::new(Document::new());
        let node = editor.document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let group = editor.document.add_group(&[node], [26080.0, 26060.0], [280.0, 270.0], 0);
        editor.select(Some(node));
        let mut state = GraphState::default();
        let none = egui::Modifiers::NONE;
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        let start = Pos2::new(26120.0, 26120.0);
        start_element_drag(&mut editor, &mut state, node, start);
        let moved = state.view().to_screen(state.rect.min, start + Vec2::new(400.0, 0.0));
        let press = egui::Event::PointerButton { pos: moved, button: PointerButton::Primary, pressed: true, modifiers: none };
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(moved), press], none);
        state.snap_lines = vec![[start, start + Vec2::splat(100.0)]];
        state.group_states.insert(group, GroupState::Outgoing);
        let escape = egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: none };
        graph_frame(&context, &mut editor, &mut state, vec![escape], none);
        assert!(state.drag.is_none());
        assert!(state.snap_lines.is_empty());
        assert!(state.group_states.is_empty());
        assert_eq!(editor.document.group_of(node), None);
    }

    #[test]
    fn modal_and_window_layers_keep_the_graphs_wheel_and_middle_button() {
        for modal in [false, true] {
            let context = themed_context();
            let mut editor = Editor::new(Document::new());
            let mut state = GraphState::default();
            let covered = std::cell::Cell::new(Rect::NOTHING);
            let frame = |events: Vec<egui::Event>, editor: &mut Editor, state: &mut GraphState| {
                let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
                let output = context.run_ui(input, |ui| {
                    show(ui, SCREEN, editor, state, &mut Vec::new());
                    if modal {
                        egui::Modal::new(egui::Id::new("blocking-modal")).show(ui.ctx(), |ui| { ui.label("Modal"); });
                    } else {
                        let mut window = egui::Window::new("Covering window");
                        window = window.fixed_pos(Pos2::new(200.0, 200.0));
                        window = window.fixed_size(Vec2::new(400.0, 400.0));
                        if let Some(shown) = window.show(ui.ctx(), |ui| { ui.label("Window"); }) {
                            covered.set(shown.response.rect);
                        }
                    }
                });
                output.drop_without_applying_deltas();
            };
            frame(Vec::new(), &mut editor, &mut state);
            frame(Vec::new(), &mut editor, &mut state);
            let point = if modal { Pos2::new(50.0, 600.0) } else { covered.get().center() };
            frame(vec![egui::Event::PointerMoved(point)], &mut editor, &mut state);
            let before = state.view();
            let wheel = egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, phase: egui::TouchPhase::Move, delta: Vec2::new(0.0, 120.0), modifiers: egui::Modifiers::NONE };
            let press = egui::Event::PointerButton { pos: point, button: PointerButton::Middle, pressed: true, modifiers: egui::Modifiers::NONE };
            frame(vec![wheel, press], &mut editor, &mut state);
            frame(vec![egui::Event::PointerMoved(point + Vec2::new(80.0, 0.0))], &mut editor, &mut state);
            assert_view_near(state.view(), before);
        }
    }

    #[test]
    fn shift_click_selects_connecting_nodes_in_both_directions() {
        let mut document = Document::new();
        let before = document.add_node("Mountain", [25600.0, 26000.0]).unwrap();
        let first = document.add_node("Blur", [26000.0, 26000.0]).unwrap();
        let middle = document.add_node("Blur", [26400.0, 26000.0]).unwrap();
        let last = document.add_node("Blur", [26800.0, 26000.0]).unwrap();
        let after = document.add_node("Blur", [27200.0, 26000.0]).unwrap();
        let separate = document.add_node("Mountain", [26000.0, 26400.0]).unwrap();
        document.connect(before, "Out", first, "In");
        document.connect(first, "Out", middle, "In");
        document.connect(middle, "Out", last, "In");
        document.connect(last, "Out", after, "In");
        let layouts: Vec<NodeLayout> = document.terrain().nodes.values().map(|node| layout_node(node, "Standard")).collect();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();

        editor.select(Some(first));
        select_clicked_node(&mut editor, &mut state, &layouts, last, egui::Modifiers::SHIFT);
        assert_eq!(editor.selection.len(), 3);
        assert!(editor.selection.contains(&first) && editor.selection.contains(&middle) && editor.selection.contains(&last));
        assert!(!editor.selection.contains(&before) && !editor.selection.contains(&after));

        editor.select(Some(last));
        select_clicked_node(&mut editor, &mut state, &layouts, first, egui::Modifiers::SHIFT);
        assert_eq!(editor.selection.len(), 3);
        assert!(editor.selection.contains(&first) && editor.selection.contains(&middle) && editor.selection.contains(&last));

        // Shift adds a disconnected node and never toggles it off.
        select_clicked_node(&mut editor, &mut state, &layouts, separate, egui::Modifiers::SHIFT);
        select_clicked_node(&mut editor, &mut state, &layouts, separate, egui::Modifiers::SHIFT);
        assert_eq!(editor.selection.len(), 4);
        assert!(editor.selection.contains(&separate));
        // Ctrl keeps its toggle behavior, including when Shift is held.
        let control_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        select_clicked_node(&mut editor, &mut state, &layouts, separate, control_shift);
        assert!(!editor.selection.contains(&separate));
    }

    #[test]
    fn the_shine_sweeps_left_to_right() {
        // Before it starts the band is off the left edge; half way it is
        // across the node; at the end off the right.
        let start = shine_stops(0.0);
        assert_eq!(start, [-0.65, -0.5, -0.49]);
        assert_eq!(shine_opacity(0.0, start), 0.0);
        let middle = shine_stops(0.25);
        assert!(shine_opacity(middle[1], middle) == SHINE_PEAK);
        assert!(shine_opacity(0.0, middle) == 0.0);
        let rising = (middle[0] + middle[1]) / 2.0;
        assert!((shine_opacity(rising, middle) - SHINE_PEAK / 2.0).abs() < 1e-5);
        let end = shine_stops(0.5);
        assert!((end[0] - 1.25).abs() < 1e-6 && (end[1] - 1.3).abs() < 1e-6 && (end[2] - 1.31).abs() < 1e-6);
        assert_eq!(shine_opacity(1.0, end), 0.0);
        let mesh = shine_mesh(Rect::from_min_size(Pos2::ZERO, Vec2::new(230.0, 80.0)), 4.0, Color32::WHITE, middle);
        assert!(mesh.vertices.len() >= 4 && mesh.indices.len().is_multiple_of(3));
    }

    #[test]
    fn snapping_follows_snap_element_to_grid() {
        assert_eq!(snap_to_unit(26012.0, 25.0), 26000.0);
        assert_eq!(snap_to_unit(26012.5, 25.0), 26000.0);
        assert_eq!(snap_to_unit(26013.0, 25.0), 26025.0);
        assert_eq!(snap_to_unit(-30.0, 25.0), -25.0);
    }

    #[test]
    fn items_snap_to_the_nearest_edges() {
        let other = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(230.0, 80.0));
        let dragged = Rect::from_min_size(Pos2::new(250.0, 10.0), Vec2::new(230.0, 80.0));
        let (shift, lines) = snap_to_items(dragged, &[other]);
        // Left against the other's right (20 away), tops level (10 away).
        assert_eq!(shift, Vec2::new(-20.0, -10.0));
        assert_eq!(lines.len(), 2);
        let far = Rect::from_min_size(Pos2::new(600.0, 300.0), Vec2::new(230.0, 80.0));
        assert_eq!(snap_to_items(far, &[other]).0, Vec2::ZERO);
    }

    #[test]
    fn connections_let_go_on_ports() {
        let dragged = |port: &str, from_output: bool, collapsed: bool| DraggedConnection { node: 1, port: port.to_string(), from_output, collapsed };
        let out = dragged("Out", true, false);
        let connect = PortDrop::Connect { from: 1, from_port: "Out".to_string(), to: 2, to_port: "In".to_string() };
        assert_eq!(port_drop(&out, 2, "In", true), connect);
        let combine = PortDrop::CombineOutputs { dragged: 1, dragged_port: "Out".to_string(), other: 2, other_port: "Wear".to_string() };
        assert_eq!(port_drop(&out, 2, "Wear", false), combine);
        assert_eq!(port_drop(&out, 2, COLLAPSED_PORT, false), PortDrop::PortsMenu { node: 2 });
        assert_eq!(port_drop(&out, 1, "Wear", false), PortDrop::Nothing);
        // Backwards out of an input: an output feeds it, an input does not.
        let input = dragged("In", false, false);
        let backwards = PortDrop::Connect { from: 2, from_port: "Out".to_string(), to: 1, to_port: "In".to_string() };
        assert_eq!(port_drop(&input, 2, "Out", false), backwards);
        assert_eq!(port_drop(&input, 2, "In", true), PortDrop::Nothing);
        // Collapsed ports onto collapsed ports: both picked in the window.
        let collapsed = dragged("Input2", false, true);
        assert_eq!(port_drop(&collapsed, 2, COLLAPSED_PORT, true), PortDrop::CreateConnection { node: 2 });
        let onto_port = PortDrop::Connect { from: 2, from_port: "Out".to_string(), to: 1, to_port: "Input2".to_string() };
        assert_eq!(port_drop(&collapsed, 2, "Out", false), onto_port);
    }

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1200.0, 800.0));

    /// One frame of the graph with these events, the modifiers held.
    fn graph_frame(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, events: Vec<egui::Event>, modifiers: egui::Modifiers) -> Vec<Command> {
        let mut held_then_events = vec![egui::Event::ModifiersChanged(modifiers)];
        held_then_events.extend(events);
        let input = egui::RawInput { screen_rect: Some(SCREEN), events: held_then_events, ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui, SCREEN, editor, state, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    /// A file dropped on the window.
    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            Ok(Vec::new())
        }
    }

    /// One frame of the graph in which `file` drops, the pointer moving to
    /// `at` first when the system says where it is.
    fn drop_frame(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, file: &str, at: Option<Pos2>) -> Vec<Command> {
        let events = at.map(egui::Event::PointerMoved).into_iter().collect();
        let dropped: egui::DroppedFileHandle = std::sync::Arc::new(Dropped(PathBuf::from(file)));
        let input = egui::RawInput { screen_rect: Some(SCREEN), events, dropped_files: vec![dropped], ..Default::default() };
        let mut commands = Vec::new();
        let output = context.run_ui(input, |ui| show(ui, SCREEN, editor, state, &mut commands));
        output.drop_without_applying_deltas();
        commands
    }

    #[test]
    fn dropped_files_land_where_they_drop_on_the_graph() {
        let file = "/maps/height.png";
        let dropped_at = |state: &GraphState, point: Pos2| {
            let canvas = state.view().to_canvas(state.rect.min, point);
            let drop = GraphCommand::DropFiles { files: vec![PathBuf::from(file)], at: [canvas.x, canvas.y] };
            vec![Command::Graph(drop)]
        };
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        // On the graph: where the pointer is.
        let point = Pos2::new(300.0, 200.0);
        let commands = drop_frame(&context, &mut editor, &mut state, file, Some(point));
        assert_eq!(commands, dropped_at(&state, point));
        // Somewhere else: not the graph's.
        let elsewhere = Pos2::new(SCREEN.right() + 50.0, 100.0);
        assert!(drop_frame(&context, &mut editor, &mut state, file, Some(elsewhere)).is_empty());
        // Where the system can't say: the middle.
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let commands = drop_frame(&context, &mut editor, &mut state, file, None);
        assert_eq!(commands, dropped_at(&state, state.rect.center()));
    }

    /// A context with the editor's fonts, which take effect a frame after
    /// they are set.
    fn themed_context() -> egui::Context {
        let context = egui::Context::default();
        theme::apply(&context);
        let input = egui::RawInput { screen_rect: Some(SCREEN), ..Default::default() };
        let output = context.run_ui(input, |_| {});
        output.drop_without_applying_deltas();
        context
    }

    /// A press at `from`, a drag to `to` and the release there.
    fn drag(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, from: Pos2, to: Pos2, modifiers: egui::Modifiers) -> Vec<Command> {
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers };
        graph_frame(context, editor, state, vec![egui::Event::PointerMoved(from)], modifiers);
        graph_frame(context, editor, state, vec![button(from, true)], modifiers);
        let past_threshold = from + Vec2::new(12.0, 0.0);
        graph_frame(context, editor, state, vec![egui::Event::PointerMoved(past_threshold)], modifiers);
        graph_frame(context, editor, state, vec![egui::Event::PointerMoved(to)], modifiers);
        graph_frame(context, editor, state, vec![button(to, false)], modifiers)
    }

    /// The screen point of a port's dot.
    fn port_dot(editor: &Editor, state: &GraphState, node: i32, port: &str, is_input: bool) -> Pos2 {
        let data = editor.document.node(node).unwrap();
        let layout = layout_node(data, editor.user.options.default_node_size());
        let dot = layout.port(port, is_input).unwrap().dot.center();
        state.view().to_screen(state.rect.min, dot)
    }

    /// The view a fit of `framed` (canvas) gives on a graph at `rect`.
    fn fitted_view(framed: Rect, rect: Rect, zoom: f32) -> GraphView {
        let fitted = (rect.width() / framed.width()).min(rect.height() / framed.height());
        let zoom = if fitted >= MIN_ZOOM { fitted.min(MAX_ZOOM) } else { zoom };
        GraphView { location: framed.center().to_vec2() - rect.size() / (2.0 * zoom), zoom }
    }

    fn assert_view_near(view: GraphView, expected: GraphView) {
        let off = (view.location - expected.location).length();
        assert!(off < 0.01 && (view.zoom - expected.zoom).abs() < 1e-6, "{view:?} is not {expected:?}");
    }

    #[test]
    fn extents_fit_the_corners_beside_the_toolbox() {
        let mut document = Document::new();
        document.add_node("Mountain", [20000.0, 26000.0]).unwrap();
        document.add_node("ErosionPro", [32000.0, 30000.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        // The floating toolbox leaves its footprint out of the fit.
        state.toolbox_rect = Rect::from_min_size(Pos2::new(10.0, 45.0), Vec2::new(36.0, 400.0));
        state.zoom_request = Some(ZoomRequest::Extents);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        // The nodes' top-left corners, 200 and then 30 round them.
        let corners = Rect::from_min_max(Pos2::new(20000.0, 26000.0), Pos2::new(32000.0, 30000.0));
        let fit = fit_rect(&state, SCREEN);
        let mut expected = fitted_view(corners.expand(230.0), fit, 1.0);
        expected.location -= (fit.min - SCREEN.min) / expected.zoom;
        assert_view_near(state.view(), expected);
    }

    #[test]
    fn extents_take_in_the_corners_of_notes_and_groups_too() {
        let mut document = Document::new();
        document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let note = document.add_note([26100.0, 29000.0], 0);
        let group = document.add_group(&[], [22000.0, 26000.0], [400.0, 300.0], 0);
        let mut editor = Editor::new(document);
        let note_corner = crate::ui::graph_notes::note_rect(&editor, 0, note).unwrap().min;
        let group_corner = editor.document.group(group).unwrap().bounds().min;
        let mut state = GraphState::default();
        let context = themed_context();
        state.zoom_request = Some(ZoomRequest::Extents);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let corners = Rect::from_points(&[Pos2::new(26100.0, 26100.0), note_corner, group_corner]);
        assert_view_near(state.view(), fitted_view(corners.expand(230.0), SCREEN, 1.0));
    }

    #[test]
    fn extents_too_wide_for_the_least_zoom_keep_the_zoom_and_centre() {
        let mut document = Document::new();
        document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        document.add_node("Mountain", [1_000_000.0, 1_000_000.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        state.view_mut().zoom = 0.5;
        state.zoom_request = Some(ZoomRequest::Extents);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let corners = Rect::from_min_max(Pos2::new(26100.0, 26100.0), Pos2::new(1_000_000.0, 1_000_000.0));
        let expected = fitted_view(corners.expand(230.0), SCREEN, 0.5);
        assert_eq!(expected.zoom, 0.5);
        assert_view_near(state.view(), expected);
    }

    #[test]
    fn selection_frames_the_selected_nodes_corners() {
        let mut document = Document::new();
        let chosen = document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        document.add_node("Mountain", [30000.0, 30000.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        editor.selection = vec![chosen];
        state.zoom_request = Some(ZoomRequest::Selection);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        // One node: its corner, 460 square round it.
        let corner = Rect::from_min_size(Pos2::new(26100.0, 26100.0), Vec2::ZERO);
        assert_view_near(state.view(), fitted_view(corner.expand(230.0), SCREEN, 1.0));
        assert!((state.view().zoom - 800.0 / 460.0).abs() < 1e-5);
    }

    #[test]
    fn panning_is_an_undo_step_that_brings_the_view_back() {
        let mut document = Document::new();
        document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let before = state.view();
        let middle = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Middle, pressed, modifiers: egui::Modifiers::NONE };
        let from = Pos2::new(600.0, 600.0);
        let to = Pos2::new(500.0, 450.0);
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(from)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![middle(from, true)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(from + Vec2::new(-12.0, 0.0))], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(to)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![middle(to, false)], egui::Modifiers::NONE);
        assert_ne!(state.view().location, before.location);
        assert_eq!(editor.document.undo_labels()[0], "Graph panned");
        // Undone, the view goes back to where the pan began.
        assert!(editor.document.undo());
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        assert_eq!(state.view().location, before.location);

        // Off, panning is no edit.
        editor.user.options.set_value("RecordGraphMovementUndos", serde_json::json!(false));
        let labels = editor.document.undo_labels().len();
        graph_frame(&context, &mut editor, &mut state, vec![middle(from, true)], egui::Modifiers::NONE);
        graph_frame(&context, &mut editor, &mut state, vec![middle(from, false)], egui::Modifiers::NONE);
        assert_eq!(editor.document.undo_labels().len(), labels);
    }

    #[test]
    fn an_output_dragged_onto_another_output_combines_the_two() {
        let mut document = Document::new();
        let first = document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let second = document.add_node("ErosionPro", [26100.0, 26400.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let from = port_dot(&editor, &state, first, "Out", false);
        let to = port_dot(&editor, &state, second, "Wear", false);
        let commands = drag(&context, &mut editor, &mut state, from, to, egui::Modifiers::NONE);
        let combine = GraphCommand::CombineOutputs { dragged: first, dragged_port: "Out".to_string(), other: second, other_port: "Wear".to_string() };
        assert_eq!(commands, vec![Command::Graph(combine)]);
    }

    #[test]
    fn collapsed_ports_dragged_onto_collapsed_ports_open_create_connection() {
        let mut document = Document::new();
        let erosion = document.add_node("ErosionPro", [26100.0, 26100.0]).unwrap();
        let combine = document.add_node("Combine", [26500.0, 26300.0]).unwrap();
        document.add_input(combine);
        for node in [erosion, combine] {
            document.set_member(node, "NodeSize", serde_json::json!("Compact"), "Node size", false);
        }
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let from = port_dot(&editor, &state, erosion, COLLAPSED_PORT, false);
        let to = port_dot(&editor, &state, combine, COLLAPSED_PORT, true);
        let commands = drag(&context, &mut editor, &mut state, from, to, egui::Modifiers::NONE);
        assert!(commands.is_empty());
        assert_eq!(state.create_connection, Some(CreateConnection::new(erosion, combine, true)));
    }

    #[test]
    fn ctrl_shift_dragging_the_graph_pushes_what_lies_right() {
        let mut document = Document::new();
        let left = document.add_node("Mountain", [26100.0, 26100.0]).unwrap();
        let right = document.add_node("Mountain", [26700.0, 26100.0]).unwrap();
        let mut editor = Editor::new(document);
        let mut state = GraphState::default();
        let context = themed_context();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), egui::Modifiers::NONE);
        let view = state.view();
        let origin = state.rect.min;
        // Between the two nodes, on empty graph.
        let from = view.to_screen(origin, Pos2::new(26450.0, 26140.0));
        let to = from + Vec2::new(150.0, 30.0);
        let pushing = egui::Modifiers { shift: true, ctrl: true, command: true, ..Default::default() };
        let commands = drag(&context, &mut editor, &mut state, from, to, pushing);
        assert!(commands.is_empty());
        let shift = view.to_canvas(origin, to).x - view.to_canvas(origin, from).x;
        assert_eq!(editor.document.node(left).unwrap().position, [26100.0, 26100.0]);
        let pushed = editor.document.node(right).unwrap().position;
        assert!((pushed[0] - (26700.0 + shift)).abs() < 1e-3 && pushed[1] == 26100.0, "{pushed:?}");
        assert_eq!(editor.document.undo_labels()[0], "Push");
        assert!(state.drag.is_none());
    }

    #[test]
    fn pushes_take_what_lies_right_of_the_pointer() {
        let mut document = Document::new();
        let left = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let right = document.add_node("Mountain", [500.0, 0.0]).unwrap();
        // A group right of the point takes its node and note along.
        let grouped = document.add_node("Blur", [800.0, 50.0]).unwrap();
        let note = document.add_note([820.0, 200.0], 0);
        let group = document.add_group(&[grouped, note], [780.0, 0.0], [300.0, 300.0], 0);
        // A node right of the point in a group left of it goes on its own.
        let straggler = document.add_node("Blur", [300.0, 400.0]).unwrap();
        let home = document.add_group(&[straggler], [100.0, 350.0], [300.0, 200.0], 0);
        let loose_note = document.add_note([400.0, 700.0], 0);
        // Other tabs stay as they are.
        let elsewhere = document.add_node("Mountain", [900.0, 0.0]).unwrap();
        document.project.terrain_mut().nodes.get_mut(&elsewhere).unwrap().set_member("GraphIndex", serde_json::json!(1));

        let pushed = elements_to_push(&document, 0, 200.0);
        let mut nodes: Vec<i32> = pushed.nodes.iter().map(|(id, _)| *id).collect();
        nodes.sort_unstable();
        let mut expected = vec![right, straggler];
        expected.sort_unstable();
        assert_eq!(nodes, expected);
        assert!(!nodes.contains(&left) && !nodes.contains(&grouped));
        assert_eq!(pushed.groups, vec![(group, [780.0, 0.0])]);
        assert!(!pushed.groups.iter().any(|(id, _)| *id == home));
        assert_eq!(pushed.notes, vec![(loose_note, [400.0, 700.0])]);
        assert!(elements_to_push(&document, 0, 2000.0).is_empty());
    }

    #[test]
    fn compact_nodes_collapse_their_extra_ports() {
        let mut document = Document::new();
        let combine = document.add_node("Combine", [0.0, 0.0]).unwrap();
        document.add_input(combine);
        let mut node = document.node(combine).unwrap().clone();
        node.set_member("NodeSize", serde_json::json!("Compact"));
        let layout = layout_node(&node, "Standard");
        let shown: Vec<&str> = layout.inputs.iter().filter(|port| !port.hidden).map(|port| port.name.as_str()).collect();
        assert_eq!(shown, vec!["In", COLLAPSED_PORT]);
        assert_eq!(layout.rect.height(), COMPACT_HEIGHT);
    }

    /// The graph amid the editor's other panels (the 3D view and its
    /// toolbar, the tab strip, the Data View, the status bar), whose
    /// widgets come before the graph's as the editor lays them out: one
    /// under the whole window stands for them.
    const GRAPH: Rect = Rect::from_min_max(Pos2::new(200.0, 300.0), Pos2::new(1100.0, 750.0));

    /// One frame of the graph at `GRAPH` with these events: the shapes
    /// painted, what of the pointer the panels around it took, and the
    /// graph's commands.
    fn frame_amid_panels(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, events: Vec<egui::Event>) -> (Vec<egui::epaint::ClippedShape>, Response, Vec<Command>) {
        let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
        let mut commands = Vec::new();
        let mut panels = None;
        let mut output = context.run_ui(input, |ui| {
            panels = Some(ui.interact(SCREEN, egui::Id::new("panels"), Sense::click_and_drag()));
            show(ui, GRAPH, editor, state, &mut commands);
        });
        let shapes = std::mem::take(&mut output.shapes);
        output.drop_without_applying_deltas();
        (shapes, panels.unwrap(), commands)
    }

    #[test]
    fn elements_panned_past_the_edge_stay_inside_the_graph() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        egui_extras::install_image_loaders(&context);
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

        // A node with its icon above the graph's top edge; a locked group
        // and an icon note with theirs left of its left edge.
        let view = state.view();
        let canvas_at = |screen: Pos2| {
            let point = view.to_canvas(GRAPH.min, screen);
            [point.x, point.y]
        };
        let node_corner = Pos2::new(GRAPH.left() + 300.0, GRAPH.top() - 40.0);
        let node = editor.document.add_node("Mountain", canvas_at(node_corner)).unwrap();
        let group_corner = Pos2::new(GRAPH.left() - 100.0, GRAPH.top() + 100.0);
        let group = editor.document.add_group(&[], canvas_at(group_corner), [300.0, 200.0], 0);
        editor.document.set_group_locked(group, true);
        let note_corner = Pos2::new(GRAPH.left() - 40.0, GRAPH.top() + 250.0);
        let note = editor.document.add_note(canvas_at(note_corner), 0);
        editor.document.toggle_note_type(note);

        // Nothing shows outside the graph.
        let (shapes, _, _) = frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        for clipped in &shapes {
            let bounds = clipped.shape.visual_bounding_rect();
            let shown = clipped.clip_rect.intersect(bounds);
            assert!(!shown.is_positive() || GRAPH.contains_rect(shown), "{shown:?} shows outside the graph");
        }

        // A click on the node's hidden part is the 3D view's.
        let hidden_part = node_corner + Vec2::new(100.0, 20.0);
        let button = |pressed: bool| egui::Event::PointerButton { pos: hidden_part, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame_amid_panels(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(hidden_part)]);
        frame_amid_panels(&context, &mut editor, &mut state, vec![button(true)]);
        let (_, panels, _) = frame_amid_panels(&context, &mut editor, &mut state, vec![button(false)]);
        assert!(panels.clicked());
        assert!(!editor.selection.contains(&node));
    }

    #[test]
    fn icons_across_any_edge_are_cut_off_there() {
        // At 200%, 100% and 55%, nodes (one dimmed), a locked group, a
        // collapsed group and an icon note sit across each of the graph's
        // edges, the nodes marked for export and one locked in the
        // preview: their icons cross every edge and none shows past it.
        for zoom in [2.0, 1.0, 0.55] {
            let mut editor = Editor::new(Document::new());
            let mut state = GraphState::default();
            let context = themed_context();
            egui_extras::install_image_loaders(&context);
            frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
            state.view_mut().zoom = zoom;
            let view = state.view();
            let default_size = editor.user.options.default_node_size().to_string();
            // The corner that puts an element `size` big (canvas) with its
            // middle at the screen point `middle`.
            let centred = |middle: Pos2, size: Vec2| {
                let corner = view.to_canvas(GRAPH.min, middle) - size / 2.0;
                [corner.x, corner.y]
            };

            let edges = [
                (GRAPH.left_top(), Vec2::new(GRAPH.width(), 0.0)),
                (GRAPH.left_bottom(), Vec2::new(GRAPH.width(), 0.0)),
                (GRAPH.left_top(), Vec2::new(0.0, GRAPH.height())),
                (GRAPH.right_top(), Vec2::new(0.0, GRAPH.height())),
            ];
            let mut nodes = Vec::new();
            for (start, along) in edges {
                for (fraction, dimmed) in [(0.15, false), (0.35, true)] {
                    let node = editor.document.add_node("Mountain", [0.0, 0.0]).unwrap();
                    let [width, height] = node_extent(editor.document.node(node).unwrap(), &default_size);
                    editor.document.move_nodes(&[(node, centred(start + along * fraction, Vec2::new(width, height)))], false);
                    if dimmed {
                        state.dimmed.insert(node);
                    }
                    nodes.push(node);
                }
                let size = Vec2::new(300.0, 200.0);
                let locked = editor.document.add_group(&[], centred(start + along * 0.55, size), [size.x, size.y], 0);
                editor.document.set_group_locked(locked, true);
                let collapsed = editor.document.add_group(&[], [0.0, 0.0], [size.x, size.y], 0);
                editor.document.toggle_group_collapsed(collapsed);
                let collapsed_size = editor.document.group(collapsed).unwrap().bounds().size();
                editor.document.move_group(collapsed, centred(start + along * 0.75, collapsed_size), false);
                let note = editor.document.add_note([0.0, 0.0], 0);
                editor.document.toggle_note_type(note);
                let note_size = crate::ui::graph_notes::note_rect(&editor, 0, note).unwrap().size();
                editor.document.set_note_vector(note, "Position", centred(start + along * 0.9, note_size), false);
            }
            editor.document.toggle_export(&nodes, nerthus_export::FileFormat::Exr);
            editor.preview_node = nodes.last().copied();
            editor.preview_locked = true;

            // Icons are the shapes drawn with a texture of their own.
            let (shapes, _, _) = frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
            let mut crossed = [false; 4];
            for clipped in &shapes {
                let bounds = clipped.shape.visual_bounding_rect();
                let shown = clipped.clip_rect.intersect(bounds);
                assert!(!shown.is_positive() || GRAPH.contains_rect(shown), "at {zoom}: {shown:?} shows outside the graph");
                if clipped.shape.texture_id() != egui::TextureId::default() {
                    crossed[0] |= bounds.top() < GRAPH.top();
                    crossed[1] |= bounds.bottom() > GRAPH.bottom();
                    crossed[2] |= bounds.left() < GRAPH.left();
                    crossed[3] |= bounds.right() > GRAPH.right();
                }
            }
            assert_eq!(crossed, [true; 4], "at {zoom}: icons cross the top, bottom, left and right edges");
        }
    }

    #[test]
    fn auto_pan_steps_towards_each_edge_the_pointer_is_near() {
        let rect = Rect::from_min_max(Pos2::new(100.0, 50.0), Pos2::new(900.0, 650.0));
        let stepped = |zoom: f32, pointer: Pos2| {
            let mut view = GraphView { location: Vec2::splat(100.0), zoom };
            auto_pan_step(&mut view, rect, pointer);
            let step = view.location - Vec2::splat(100.0);
            // To a thousandth, past the float's rounding.
            (step * 1000.0).round() / 1000.0
        };
        // In the middle, or just over 32 from the edge: no step.
        assert_eq!(stepped(1.0, rect.center()), Vec2::ZERO);
        assert_eq!(stepped(1.0, Pos2::new(rect.left() + 32.5, 300.0)), Vec2::ZERO);
        // Within 32 of the left edge, or past the right: 0.15 / (zoom * 2)
        // canvas units that way, however far past.
        assert_eq!(stepped(1.0, Pos2::new(rect.left() + 32.0, 300.0)), Vec2::new(-0.075, 0.0));
        assert_eq!(stepped(0.5, Pos2::new(rect.right() + 500.0, 300.0)), Vec2::new(0.15, 0.0));
        // In a corner, both ways.
        assert_eq!(stepped(1.0, Pos2::new(rect.right() - 5.0, rect.top() + 5.0)), Vec2::new(0.075, -0.075));
        // Kept on the canvas, the graph's size in screen points.
        let mut view = GraphView { location: Vec2::new(0.02, CANVAS_EXTENTS), zoom: 0.25 };
        auto_pan_step(&mut view, rect, rect.center());
        assert_eq!(view.location, Vec2::new(0.02, CANVAS_EXTENTS - rect.height()));
        auto_pan_step(&mut view, rect, Pos2::new(rect.left(), 300.0));
        assert_eq!(view.location.x, 0.0);
    }

    #[test]
    fn a_node_dragged_near_the_edge_steps_the_view_on_each_move_only() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        let view = state.view();
        let corner = view.to_canvas(GRAPH.min, GRAPH.center());
        let node = editor.document.add_node("Mountain", [corner.x, corner.y]).unwrap();

        // Taken by its title and moved well inside the graph: the view
        // stays.
        let title = GRAPH.center() + Vec2::new(100.0, 15.0);
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame_amid_panels(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(title)]);
        frame_amid_panels(&context, &mut editor, &mut state, vec![button(title, true)]);
        frame_amid_panels(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(title + Vec2::new(40.0, 0.0))]);
        assert_eq!(state.view(), view);

        // Moved past the right edge, over the panel beside the graph: a
        // step for each move, none while the pointer rests.
        let past = Pos2::new(GRAPH.right() + 60.0, title.y);
        let moves = vec![egui::Event::PointerMoved(past - Vec2::X), egui::Event::PointerMoved(past)];
        frame_amid_panels(&context, &mut editor, &mut state, moves);
        let stepped = state.view().location;
        assert!((stepped - view.location - Vec2::new(0.15, 0.0)).length() < 0.01, "{stepped:?}");
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        assert_eq!(state.view().location, stepped);

        // Let go there, the node is where the pointer took it, past the
        // graph's edge.
        frame_amid_panels(&context, &mut editor, &mut state, vec![button(past, false)]);
        let layout = layout_node(editor.document.node(node).unwrap(), editor.user.options.default_node_size());
        let screen = state.view().rect_to_screen(GRAPH.min, layout.rect);
        assert!(screen.left() > GRAPH.right() - 100.0 && screen.right() > GRAPH.right(), "{screen:?}");
    }

    #[test]
    fn hidden_parts_leave_hover_clicks_and_drags_to_the_panels_around() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

        // A node across the top edge, one just right of the graph (inside
        // the 40 nodes are kept for), one with its input past the left
        // edge, a group across the left edge and a text note across the
        // bottom right corner, and a node well inside.
        let view = state.view();
        let canvas_at = |x: f32, y: f32| {
            let point = view.to_canvas(GRAPH.min, Pos2::new(x, y));
            [point.x, point.y]
        };
        let document = &mut editor.document;
        document.add_node("Mountain", canvas_at(500.0, 260.0)).unwrap();
        document.add_node("Mountain", canvas_at(1120.0, 450.0)).unwrap();
        let with_input = document.add_node("ErosionPro", canvas_at(170.0, 630.0)).unwrap();
        document.add_group(&[], canvas_at(100.0, 320.0), [300.0, 270.0], 0);
        let note = document.add_note(canvas_at(950.0, 650.0), 0);
        document.set_note_vector(note, "Size", [200.0, 120.0], false);
        let inside = document.add_node("Mountain", canvas_at(600.0, 450.0)).unwrap();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        let undo_steps = editor.document.undo_labels().len();

        let hidden_input = port_dot(&editor, &state, with_input, "In", true);
        let hidden_parts = [
            ("the node across the top", Pos2::new(600.0, 280.0)),
            ("the node beside the graph", Pos2::new(1140.0, 470.0)),
            ("the input past the left edge", hidden_input),
            ("the group's header", Pos2::new(150.0, 330.0)),
            ("the group's menu button", Pos2::new(120.0, 338.0)),
            ("the group's resize thumb", Pos2::new(110.0, 580.0)),
            ("the note", Pos2::new(1050.0, 760.0)),
            ("the note's resize thumb", Pos2::new(1140.0, 760.0)),
        ];
        let button = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut commands = Vec::new();
        for (part, point) in hidden_parts {
            assert!(!GRAPH.contains(point), "{part} shows in the graph");
            let mut frame = |events: Vec<egui::Event>| {
                let (_, panels, graph_commands) = frame_amid_panels(&context, &mut editor, &mut state, events);
                commands.extend(graph_commands);
                panels
            };
            // Hovered, without the graph's tooltip; clicked; dragged.
            assert!(frame(vec![egui::Event::PointerMoved(point)]).hovered(), "{part} takes the hover");
            frame(vec![button(point, true)]);
            assert!(frame(vec![button(point, false)]).clicked(), "{part} takes the click");
            frame(vec![button(point, true)]);
            let away = point + Vec2::new(0.0, -30.0);
            assert!(frame(vec![egui::Event::PointerMoved(away)]).dragged(), "{part} takes the drag");
            frame(vec![button(away, false)]);
        }
        assert!(commands.is_empty(), "{commands:?}");
        assert!(editor.selection.is_empty() && state.selected_groups.is_empty() && state.selected_note.is_none());
        assert!(state.drag.is_none() && state.popup.is_none());
        assert_eq!(editor.document.undo_labels().len(), undo_steps);

        // A connection let go on the hidden input joins nothing.
        let from = port_dot(&editor, &state, inside, "Out", false);
        let mut frame = |events: Vec<egui::Event>| frame_amid_panels(&context, &mut editor, &mut state, events).2;
        frame(vec![egui::Event::PointerMoved(from)]);
        frame(vec![button(from, true)]);
        frame(vec![egui::Event::PointerMoved(from + Vec2::new(12.0, 0.0))]);
        frame(vec![egui::Event::PointerMoved(hidden_input)]);
        assert!(frame(vec![button(hidden_input, false)]).is_empty());
        assert!(state.drag.is_none() && state.popup.is_none());
    }

    /// Rests the pointer at `point` until the tooltip saying `text` shows:
    /// where it shows.
    fn tooltip_at(context: &egui::Context, editor: &mut Editor, state: &mut GraphState, point: Pos2, text: &str) -> Rect {
        frame_amid_panels(context, editor, state, vec![egui::Event::PointerMoved(point)]);
        for _ in 0..120 {
            let (shapes, _, _) = frame_amid_panels(context, editor, state, Vec::new());
            let tooltips: Vec<Rect> = context.memory(|memory| {
                let layers = memory.areas().visible_layer_ids().into_iter().filter(|layer| layer.order == egui::Order::Tooltip);
                layers.filter_map(|layer| memory.area_rect(layer.id)).collect()
            });
            let texts = shapes.iter().filter_map(|clipped| match &clipped.shape {
                Shape::Text(shape) if shape.galley.text() == text => Some(clipped.shape.visual_bounding_rect()),
                _ => None,
            });
            let shown = texts.into_iter().find_map(|bounds| tooltips.iter().copied().find(|tooltip| tooltip.contains_rect(bounds)));
            if let Some(tooltip) = shown {
                return tooltip;
            }
        }
        panic!("no tooltip says {text:?}");
    }

    #[test]
    fn tooltips_show_beside_the_pointer() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

        // At the closest zoom an Erosion node, a group and an icon note are
        // each hundreds of points across.
        state.view_mut().zoom = MAX_ZOOM;
        let view = state.view();
        let canvas_at = |x: f32, y: f32| {
            let point = view.to_canvas(GRAPH.min, Pos2::new(x, y));
            [point.x, point.y]
        };
        let erosion = editor.document.add_node("ErosionPro", canvas_at(220.0, 310.0)).unwrap();
        let group = editor.document.add_group(&[], canvas_at(720.0, 320.0), [170.0, 100.0], 0);
        let note = editor.document.add_note(canvas_at(760.0, 560.0), 0);
        editor.document.toggle_note_type(note);
        editor.document.set_note_text(note, "Carve the valley first");
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

        // Rested on the node's middle, the group header's right end and the
        // note's lower right, far from their lower left corners.
        let view = state.view();
        let node = editor.document.node(erosion).unwrap();
        let node_box = view.rect_to_screen(GRAPH.min, layout_node(node, editor.user.options.default_node_size()).rect);
        let group_data = editor.document.group(group).unwrap();
        let header = view.rect_to_screen(GRAPH.min, group_data.bounds());
        let note_box = view.rect_to_screen(GRAPH.min, crate::ui::graph_notes::note_rect(&editor, 0, note).unwrap());
        let rests = [
            ("the node", node_box.center(), node_tooltip(&editor, erosion).text),
            ("the group", Pos2::new(header.right() - 20.0, header.top() + 20.0), group_data.name.clone()),
            ("the note", note_box.max - Vec2::splat(10.0), "Carve the valley first".to_string()),
        ];
        for (element, point, text) in rests {
            assert!(GRAPH.contains(point), "{element} rests outside the graph");
            let tooltip = tooltip_at(&context, &mut editor, &mut state, point, &text);
            assert!(tooltip.distance_to_pos(point) <= 16.0, "{element}'s tooltip {tooltip:?} is far from the pointer {point:?}");
            let canvas = state.view().to_canvas(GRAPH.min, point);
            let search = SearchPopup::new(SearchMode::Nodes { source: NodeSource::None }, point, [canvas.x, canvas.y]);
            state.popup = Some(GraphPopup::Search(search));
            for _ in 0..3 {
                frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
            }
            let tooltip_visible = context.memory(|memory| {
                memory.areas().visible_layer_ids().iter().any(|layer| layer.order == egui::Order::Tooltip)
            });
            assert!(!tooltip_visible, "{element}'s tooltip covers the open search");
            state.popup = None;
        }
    }

    #[test]
    fn ports_show_their_display_names_and_required_inputs_at_every_node_size() {
        let mut document = Document::new();
        let id = document.add_node("ErosionPro", [26000.0, 26000.0]).unwrap();
        let node = document.project.terrain_mut().nodes.get_mut(&id).unwrap();
        let input = node.ports.iter_mut().find(|port| port.is_input() && port.name != "Mask").unwrap();
        input.port_type = "PrimaryIn, Required".to_string();
        input.extra.insert("DisplayName".to_string(), Value::from("Terrain Input"));
        let input_name = input.name.clone();
        let editor = Editor::new(document);
        let node = editor.document.node(id).unwrap();
        for size in ["Standard", "Compact", "Small"] {
            let layout = layout_node(node, size);
            let input = layout.port(&input_name, true).unwrap();
            let output = layout.port("Out", false).unwrap();
            assert_eq!(port_tooltip(&editor, id, input), "Terrain Input (Required)");
            assert_eq!(port_tooltip(&editor, id, output), "Out");
            if let Some(collapsed) = layout.ports().find(|port| port.collapsed) {
                assert_eq!(port_tooltip(&editor, id, collapsed), "All Ports");
            }
        }
    }

    #[test]
    fn node_tooltips_follow_the_name_time_status_and_error_runs() {
        let mut document = Document::new();
        let id = document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let name = document.node(id).unwrap().name.clone();
        let mut editor = Editor::new(document);
        let status = crate::editor::NodeStatus {
            state: NodeState::Error,
            milliseconds: 1250.0,
            error: Some("Missing input".to_string()),
            ..Default::default()
        };
        editor.statuses.insert(id, status);
        let tooltip = node_tooltip(&editor, id);
        assert_eq!(tooltip.text, format!("{name}\nBuild time:  1 second\nStatus: Error Missing input"));
        assert_eq!(tooltip.sections[0].format.font_id, theme::semibold(12.0));
        let state_start = egui::text::ByteIndex(tooltip.text.find("Error").unwrap());
        let state_section = tooltip.sections.iter().find(|section| section.byte_range.contains(&state_start)).unwrap();
        assert_eq!(state_section.format.font_id, theme::semibold(12.0));
        assert_eq!(tooltip.sections.last().unwrap().format.color, theme::ACCENT_ORANGE);
        editor.statuses.get_mut(&id).unwrap().building = true;
        assert!(node_tooltip(&editor, id).text.contains("Status: Is Building"));
    }

    /// The names painted, each where its galley starts, how wide it is,
    /// its rows and its font's size.
    fn painted_names(shapes: &[egui::epaint::ClippedShape], name: &str) -> Vec<(Pos2, f32, usize, f32)> {
        let texts = shapes.iter().filter_map(|clipped| match &clipped.shape {
            Shape::Text(text) if text.galley.text() == name => {
                let size = text.galley.job.sections[0].format.font_id.size;
                Some((text.pos, text.galley.size().x, text.galley.rows.len(), size))
            }
            _ => None,
        });
        texts.collect()
    }

    #[test]
    fn names_keep_their_place_zoomed_out_in_a_24_font() {
        // A standard node with a long name, a compact one, a small one and
        // a small one with a Mask port (Final in Alpine Valley), close in
        // and zoomed out (simplified).
        let long_name = "A very long node name with spaces that goes on and on past the edge of any node box";
        for zoom in [1.0, 0.37, 0.2, 0.05] {
            let mut editor = Editor::new(Document::new());
            let mut state = GraphState::default();
            let context = themed_context();
            frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
            state.view_mut().zoom = zoom;
            let view = state.view();
            let canvas_at = |x: f32, y: f32| {
                let point = view.to_canvas(GRAPH.min, Pos2::new(x, y));
                [point.x, point.y]
            };
            let erosion = editor.document.add_node("ErosionPro", canvas_at(300.0, 350.0)).unwrap();
            let white = editor.document.add_node("Mountain", canvas_at(560.0, 350.0)).unwrap();
            let peak = editor.document.add_node("Mountain", canvas_at(760.0, 350.0)).unwrap();
            let final_node = editor.document.add_node("Combine", canvas_at(900.0, 350.0)).unwrap();
            editor.document.set_member(white, "NodeSize", serde_json::json!("Compact"), "Node size", false);
            for small in [peak, final_node] {
                editor.document.set_member(small, "NodeSize", serde_json::json!("Small"), "Node size", false);
            }
            editor.document.rename(erosion, long_name);
            editor.document.rename(white, "White");
            editor.document.rename(peak, "Peak");
            editor.document.rename(final_node, "Final");
            let (shapes, _, _) = frame_amid_panels(&context, &mut editor, &mut state, Vec::new());

            let simplified = zoom < SIMPLIFIED_ZOOM;
            let screen_box = |node: i32| view.rect_to_screen(GRAPH.min, node_rect(&editor, node));
            let size = |normal: f32| if simplified { 24.0 * zoom } else { normal * zoom };
            let height = |font_size: f32| line_height(&context, &theme::semibold(font_size));
            let one_name = |name: &str| {
                let names = painted_names(&shapes, name);
                assert_eq!(names.len(), 1, "{name} at {zoom}: {names:?}");
                names[0]
            };

            // 50, 14 in, at most 100 wide, one line cut short.
            let (start, width, rows, font_size) = one_name(long_name);
            let node_box = screen_box(erosion);
            assert_eq!((rows, font_size), (1, size(13.0)), "at {zoom}");
            assert!(start.distance(node_box.min + Vec2::new(50.0, 14.0) * zoom) < 0.01, "at {zoom}: {start:?}");
            assert!(width <= 100.0 * zoom + 0.01, "at {zoom}: {width}");

            // 80 in and centred down the node, at most 65 wide.
            let (start, width, rows, font_size) = one_name("White");
            let node_box = screen_box(white);
            assert_eq!((rows, font_size), (1, size(13.0)), "at {zoom}");
            let top = node_box.center().y - height(font_size) / 2.0;
            assert!(start.distance(Pos2::new(node_box.left() + 80.0 * zoom, top)) < 0.01, "at {zoom}: {start:?}");
            assert!(width <= 65.0 * zoom + 0.01, "at {zoom}: {width}");

            // 4 in, its foot 16 below the box, at most 60 wide.
            let (start, width, rows, font_size) = one_name("Peak");
            let node_box = screen_box(peak);
            assert_eq!((rows, font_size), (1, size(10.0)), "at {zoom}");
            let top = node_box.bottom() + 16.0 * zoom - height(font_size);
            assert!(start.distance(Pos2::new(node_box.left() + 4.0 * zoom, top)) < 0.01, "at {zoom}: {start:?}");
            assert!(width <= 60.0 * zoom + 0.01, "at {zoom}: {width}");

            // A small node gives its name up to its Mask port.
            assert!(painted_names(&shapes, "Final").is_empty(), "Final shows at {zoom}");
        }
    }

    #[test]
    fn small_node_ports_take_the_pointer_by_their_element_and_hit_grid() {
        // A small Combine, like Final in Alpine Valley: two inputs, which
        // fill both rows on the left, and an output.
        let mut document = Document::new();
        let combine = document.add_node("Combine", [26100.0, 26100.0]).unwrap();
        document.set_member(combine, "NodeSize", serde_json::json!("Small"), "Node size", false);
        let mut editor = Editor::new(document);
        let layout = layout_node(editor.document.node(combine).unwrap(), editor.user.options.default_node_size());
        assert_eq!((layout.inputs.len(), layout.outputs.len()), (2, 1));

        // The rows 14 down and centred in the rest, the elements 5 in.
        let rect = layout.rect;
        let first_row = rect.top() + 14.0 + (64.0 - 14.0 - 2.0 * 16.0) / 2.0;
        let input = &layout.inputs[0];
        let output = &layout.outputs[0];
        assert_eq!(input.element, Rect::from_min_size(Pos2::new(rect.left() + 5.0, first_row + 2.0), Vec2::new(20.0, 12.0)));
        assert_eq!(output.element, Rect::from_min_size(Pos2::new(rect.right() - 25.0, first_row + 2.0), Vec2::new(20.0, 12.0)));
        // Each takes the pointer over its element and the 14 of its hit
        // grid (20 high, to 4 past the element) on the dot's side.
        let middle = input.element.center().y;
        assert_eq!(input.hit, [input.element, Rect::from_min_max(Pos2::new(rect.left() + 5.0, middle - 10.0), Pos2::new(rect.left() + 19.0, middle + 10.0))]);
        assert_eq!(output.hit, [output.element, Rect::from_min_max(Pos2::new(rect.right() - 15.0, middle - 10.0), Pos2::new(rect.right() - 1.0, middle + 10.0))]);
        assert!(input.takes(input.dot.center()) && output.takes(output.dot.center()));
        assert!(!input.takes(Pos2::new(rect.left() + 22.0, middle - 8.0)), "the grid's corner past its 14");
        // The two sides leave the middle to the node.
        assert!(!input.takes(rect.center()) && !output.takes(rect.center()));
        // The collapsed stand-in and the Mask port are never "small".
        let mut many = Document::new();
        let mixer = many.add_node("Mixer", [26100.0, 26100.0]).unwrap();
        many.set_member(mixer, "NodeSize", serde_json::json!("Small"), "Node size", false);
        let many_layout = layout_node(many.node(mixer).unwrap(), "Standard");
        if let Some(stand_in) = many_layout.inputs.iter().find(|port| port.collapsed) {
            assert_eq!(stand_in.hit[1].width(), 24.0);
        }
        let mask = layout.mask.as_ref().unwrap();
        assert_eq!(mask.hit[1], Rect::from_min_max(Pos2::new(mask.element.left(), mask.element.center().y - 10.0), Pos2::new(mask.element.right() + 4.0, mask.element.center().y + 10.0)));

        // A right-click in the middle, beside it in the output's row, or
        // on a port's dot opens the node's menu.
        let none = egui::Modifiers::NONE;
        let output_row = layout.outputs[0].dot.center().y;
        let points = [
            ("the middle", layout.rect.center()),
            ("the output's row", Pos2::new(layout.rect.center().x + 8.0, output_row)),
            ("an input's dot", layout.inputs[1].dot.center()),
            ("the output's dot", layout.outputs[0].dot.center()),
        ];
        for (place, at) in points {
            let context = themed_context();
            let mut state = GraphState::default();
            editor.select(None);
            graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
            let point = state.view().to_screen(state.rect.min, at);
            let button = |pressed: bool| egui::Event::PointerButton { pos: point, button: PointerButton::Secondary, pressed, modifiers: none };
            graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(point)], none);
            graph_frame(&context, &mut editor, &mut state, vec![button(true)], none);
            graph_frame(&context, &mut editor, &mut state, vec![button(false)], none);
            graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
            assert!(egui::Popup::is_any_open(&context), "no menu opens on {place}");
            assert_eq!(editor.selection, vec![combine], "the menu on {place} is not the node's");
        }
    }

    /// The text in the rename popup.
    fn rename_text(context: &egui::Context) -> Option<String> {
        slider_popups::typing_text(context, rename_popup_id())
    }

    #[test]
    fn the_rename_popup_takes_enter_and_escape() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        let none = egui::Modifiers::NONE;
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        let view = state.view();
        let canvas_at = |x: f32, y: f32| {
            let point = view.to_canvas(state.rect.min, Pos2::new(x, y));
            [point.x, point.y]
        };
        let mountain = editor.document.add_node("Mountain", canvas_at(300.0, 200.0)).unwrap();
        let blur = editor.document.add_node("Blur", canvas_at(700.0, 200.0)).unwrap();
        let name = |editor: &Editor, node: i32| editor.document.node(node).unwrap().name.clone();
        let (mountain_name, blur_name) = (name(&editor, mountain), name(&editor, blur));
        let key = |key: Key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: none };
        let typed = |text: &str| egui::Event::Text(text.to_string());

        // F2 opens the popup on the name, all of it selected (`SelectAll`),
        // so typing replaces it; Enter takes what was typed.
        state.start_renaming(&editor, mountain);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        assert_eq!(rename_text(&context), Some(mountain_name.clone()));
        graph_frame(&context, &mut editor, &mut state, vec![typed("Q")], none);
        assert_eq!(rename_text(&context).as_deref(), Some("Q"));
        let commands = graph_frame(&context, &mut editor, &mut state, vec![key(Key::Enter)], none);
        let renamed = GraphCommand::RenameNode { node: mountain, name: "Q".to_string() };
        assert_eq!(commands, vec![Command::Graph(renamed)]);
        assert_eq!(state.renaming, None);

        // Escape closes it and keeps the name.
        state.start_renaming(&editor, blur);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        graph_frame(&context, &mut editor, &mut state, vec![typed("y")], none);
        assert_eq!(rename_text(&context).as_deref(), Some("y"));
        let commands = graph_frame(&context, &mut editor, &mut state, vec![key(Key::Escape)], none);
        assert!(commands.is_empty(), "{commands:?}");
        assert_eq!(state.renaming, None);

        // A press on the graph closes it (`StaysOpen` off), taking the
        // text only with Accept values when clicking outside of editable
        // field.
        let background = Pos2::new(600.0, 600.0);
        let button = |pressed: bool| egui::Event::PointerButton { pos: background, button: PointerButton::Primary, pressed, modifiers: none };
        for accept in [false, true] {
            theme::set_settings(&context, theme::InterfaceSettings { accept_on_lost_focus: accept, ..Default::default() });
            state.start_renaming(&editor, mountain);
            graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
            graph_frame(&context, &mut editor, &mut state, vec![typed("Zed")], none);
            graph_frame(&context, &mut editor, &mut state, vec![egui::Event::PointerMoved(background)], none);
            let commands = graph_frame(&context, &mut editor, &mut state, vec![button(true)], none);
            graph_frame(&context, &mut editor, &mut state, vec![button(false)], none);
            let expected = if accept { vec![Command::Graph(GraphCommand::RenameNode { node: mountain, name: "Zed".to_string() })] } else { Vec::new() };
            assert_eq!(commands, expected, "accept on lost focus: {accept}");
            assert_eq!(state.renaming, None);
            assert_eq!(rename_text(&context), None);
        }

        // Renaming another node while the popup is still open starts on
        // that node's name, not on what was typed for this one.
        state.start_renaming(&editor, blur);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        graph_frame(&context, &mut editor, &mut state, vec![typed("Q")], none);
        state.start_renaming(&editor, mountain);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        assert_eq!(state.renaming, Some(Renamed::Node(mountain)));
        assert_eq!(rename_text(&context), Some(mountain_name.clone()));
        assert_ne!(blur_name, mountain_name);

        // A reset closes it.
        state.reset();
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        assert_eq!(state.renaming, None);
        graph_frame(&context, &mut editor, &mut state, Vec::new(), none);
        assert_eq!(rename_text(&context), None);
    }

    #[test]
    fn f2_opens_the_rename_popup_on_the_selected_node_s_name() {
        let mut editor = Editor::new(Document::new());
        let mut ui_state = crate::ui::UiState::default();
        let mut window = bevy::prelude::Window::default();
        let context = themed_context();
        let none = egui::Modifiers::NONE;
        graph_frame(&context, &mut editor, &mut ui_state.graph, Vec::new(), none);
        let view = ui_state.graph.view();
        let canvas_at = |x: f32, y: f32| {
            let point = view.to_canvas(ui_state.graph.rect.min, Pos2::new(x, y));
            [point.x, point.y]
        };
        let erosion = editor.document.add_node("Erosion", canvas_at(300.0, 200.0)).unwrap();
        let snow = editor.document.add_node("Snow", canvas_at(700.0, 200.0)).unwrap();
        let erosion_name = editor.document.node(erosion).unwrap().name.clone();
        let snow_name = editor.document.node(snow).unwrap().name.clone();

        editor.select(Some(erosion));
        crate::ui::commands::run(vec![Command::Rename], &mut editor, &mut ui_state, &mut window);
        graph_frame(&context, &mut editor, &mut ui_state.graph, Vec::new(), none);
        assert_eq!(rename_text(&context), Some(erosion_name));
        graph_frame(&context, &mut editor, &mut ui_state.graph, vec![egui::Event::Text("Q".into())], none);
        assert_eq!(rename_text(&context).as_deref(), Some("Q"));

        // F2 on another node takes that node's name, whatever was typed.
        editor.select(Some(snow));
        crate::ui::commands::run(vec![Command::Rename], &mut editor, &mut ui_state, &mut window);
        graph_frame(&context, &mut editor, &mut ui_state.graph, Vec::new(), none);
        assert_eq!(ui_state.graph.renaming, Some(Renamed::Node(snow)));
        assert_eq!(rename_text(&context), Some(snow_name));
    }

    /// The text shapes painted that read `text`, in paint order: where
    /// each is in `shapes`, and its font's size.
    fn texts_reading(shapes: &[egui::epaint::ClippedShape], text: &str) -> Vec<(usize, f32)> {
        let texts = shapes.iter().enumerate().filter_map(|(index, clipped)| match &clipped.shape {
            Shape::Text(shape) if shape.galley.text() == text => Some((index, shape.galley.job.sections[0].format.font_id.size)),
            _ => None,
        });
        texts.collect()
    }

    #[test]
    fn the_rename_popup_opens_ten_in_from_the_node_at_100_percent() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        // At 120%, the popup's sizes are a sixth smaller in the editor's
        // points; areas sit on whole pixels.
        let scale = 1.2;
        let pixel = 1.0 / scale;
        context.set_zoom_factor(scale);
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        let standard = editor.document.add_node("Mountain", [26000.0, 26000.0]).unwrap();
        let small = editor.document.add_node("Combine", [26400.0, 26000.0]).unwrap();
        editor.document.set_member(small, "NodeSize", serde_json::json!("Small"), "Node size", false);
        for (node, width) in [(standard, STANDARD_WIDTH), (small, RENAME_MIN_WIDTH)] {
            let name = editor.document.node(node).unwrap().name.clone();
            let rect = node_rect(&editor, node);
            let id = rename_popup_id();
            for zoom in [0.25, 0.5, 1.0, 2.0] {
                // The node's corner a little way into the graph.
                let view = GraphView { location: rect.min.to_vec2() - Vec2::splat(40.0) / zoom, zoom };
                state.views.insert(state.tab, view);
                state.start_renaming(&editor, node);
                frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
                let (shapes, _, _) = frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
                assert_eq!(state.view(), view, "at {zoom}: the view stays put");

                // 10 in from the node's corner in the node's units, as wide
                // as the node unzoomed (96 at least), at 100%.
                let popup = context.memory(|memory| memory.area_rect(id.with("area"))).expect("the popup shows");
                let corner = view.to_screen(GRAPH.min, rect.min) + Vec2::splat(10.0 * zoom);
                assert!(popup.min.distance(corner) <= 1.5 * pixel, "at {zoom}: {popup:?} is not at {corner:?}");
                assert!((popup.width() - width / scale).abs() <= pixel, "at {zoom}: {popup:?}");
                let (_, size) = *texts_reading(&shapes, &name).last().expect("the popup shows the name");
                assert!((size - RENAME_FONT_SIZE / scale).abs() < 1e-4, "at {zoom}: {size}");
                state.stop_renaming();
            }
        }
    }

    #[test]
    fn the_rename_popup_shows_over_the_toolbox_and_past_the_graph() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        let toolbox = Rect::from_min_size(GRAPH.min, Vec2::new(150.0, GRAPH.height()));
        state.toolbox_rect = toolbox;
        let toolbox_fill = Color32::from_rgb(1, 2, 3);
        // The graph at `GRAPH` and the toolbox drawn over its corner after
        // it, as the graph area does: a fill and a button.
        let frame = |editor: &mut Editor, state: &mut GraphState, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(SCREEN), events, ..Default::default() };
            let mut commands = Vec::new();
            let mut clicked = false;
            let mut output = context.run_ui(input, |ui| {
                show(ui, GRAPH, editor, state, &mut commands);
                ui.painter().rect_filled(toolbox, 0.0, toolbox_fill);
                clicked = ui.interact(toolbox, egui::Id::new("toolbox"), Sense::click()).clicked();
            });
            let shapes = std::mem::take(&mut output.shapes);
            output.drop_without_applying_deltas();
            (shapes, clicked)
        };
        // A node whose corner is left of the graph: the popup starts
        // outside it, over the panel beside it, and runs on over the
        // toolbox.
        let view = state.view();
        let corner = view.to_canvas(GRAPH.min, Pos2::new(GRAPH.left() - 60.0, GRAPH.top() + 100.0));
        let node = editor.document.add_node("Mountain", [corner.x, corner.y]).unwrap();
        let name = editor.document.node(node).unwrap().name.clone();
        let id = rename_popup_id();
        state.start_renaming(&editor, node);
        frame(&mut editor, &mut state, Vec::new());
        let (shapes, _) = frame(&mut editor, &mut state, Vec::new());
        assert_eq!(state.view(), view, "no pan to bring it into sight");
        let popup = context.memory(|memory| memory.area_rect(id.with("area"))).unwrap();
        assert!(popup.left() < GRAPH.left() && popup.intersects(toolbox), "{popup:?}");
        let text = shapes.iter().rev().find(|clipped| matches!(&clipped.shape, Shape::Text(shape) if shape.galley.text() == name)).unwrap();
        assert!(text.clip_rect.left() < GRAPH.left(), "the popup is cut off at the graph");

        // It is painted over the toolbox: the name in it comes after the
        // toolbox's fill.
        let fill = shapes.iter().position(|clipped| matches!(&clipped.shape, Shape::Rect(rect) if rect.fill == toolbox_fill)).unwrap();
        let (last_name, _) = *texts_reading(&shapes, &name).last().unwrap();
        assert!(last_name > fill, "the toolbox covers the popup");

        // A click on it where it lies over the toolbox is the popup's: it
        // keeps the keyboard, and the toolbox takes no click.
        let point = Pos2::new(GRAPH.left() + 30.0, popup.center().y);
        let button = |pressed: bool| egui::Event::PointerButton { pos: point, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        let mut toolbox_clicked = false;
        for events in [vec![egui::Event::PointerMoved(point)], vec![button(true)], vec![button(false)], Vec::new()] {
            toolbox_clicked |= frame(&mut editor, &mut state, events).1;
        }
        assert!(!toolbox_clicked);
        assert_eq!(state.renaming, Some(Renamed::Node(node)));
        assert!(context.memory(|memory| memory.has_focus(id.with("text"))));
    }

    #[test]
    fn the_group_rename_popup_opens_ten_in_from_the_group() {
        let mut editor = Editor::new(Document::new());
        let mut state = GraphState::default();
        let context = themed_context();
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        state.view_mut().zoom = 2.0;
        let view = state.view();
        let corner = view.to_canvas(GRAPH.min, GRAPH.min + Vec2::splat(40.0));
        let group = editor.document.add_group(&[], [corner.x, corner.y], [120.0, 100.0], 0);
        let name = editor.document.group(group).unwrap().name.clone();
        state.start_renaming_group(group, name.clone());
        frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        let (shapes, _, _) = frame_amid_panels(&context, &mut editor, &mut state, Vec::new());
        assert_eq!(state.view(), view, "no pan to bring it into sight");

        let id = rename_popup_id();
        let popup = context.memory(|memory| memory.area_rect(id.with("area"))).expect("the popup shows");
        let bounds = editor.document.group(group).unwrap().bounds();
        let expected = view.to_screen(GRAPH.min, bounds.min) + Vec2::splat(20.0);
        assert!(popup.min.distance(expected) < 1.0, "{popup:?} is not at {expected:?}");
        assert!((popup.width() - bounds.width()).abs() < 1.0, "{popup:?}");
        let (_, size) = *texts_reading(&shapes, &name).last().unwrap();
        assert_eq!(size, RENAME_FONT_SIZE);

        // Enter takes the name.
        let typed = egui::Event::Text("Ridges".to_string());
        frame_amid_panels(&context, &mut editor, &mut state, vec![typed]);
        let enter = egui::Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame_amid_panels(&context, &mut editor, &mut state, vec![enter]);
        assert_eq!(editor.document.group(group).unwrap().name, "Ridges");
        assert_eq!(state.renaming, None);
    }

    fn node_rect(editor: &Editor, id: i32) -> Rect {
        layout_node(editor.document.node(id).unwrap(), editor.user.options.default_node_size()).rect
    }

}
