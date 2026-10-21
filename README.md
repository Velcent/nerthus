<div align="center" id = "top">
  <img alt="logo" height="200px" src="https://raw.githubusercontent.com/danemadsen/nerthus/refs/heads/main/crates/nerthus-app/assets/nerthus.svg">
</div>

# Nerthus

Nerthus is a clean room reimplementation of Gaea 2.3.1.0, the node-based terrain
generator, written in Rust. It opens and saves Gaea 2 `.terrain` projects, and
projects it saves open in Gaea. Its goal is to build them into the
same heightfields, masks and colour maps that Gaea does.

Nerthus is an independent project. It is not affiliated with or endorsed by
the makers of Gaea.

![The Nerthus editor](media/editor.png)

## A look around

A terrain is built as a graph of nodes. The viewport shows the selected node in
3D as it builds, and the property panel beside it changes the node's settings.
Most terrains take the same few steps: a shape to start from, simulations that
wear it down, detail on its surface, then colour.

![The Volcanic Isle example built up: Volcano, then Lichtenberg and ErosionPro, then Sea, then its colour](media/workflow.gif)

### Create

Primitives and terrain nodes make the first shape: mountains, ranges, ridges,
canyons, volcanoes, islands, dunes, craters and noises.

![Volcano, MountainRange, DuneSea and CraterField](media/create.jpg)

### Simulate

Erosion, rivers, lakes, the sea, thermal erosion, snow and glaciers work the
shape over.

![ErosionPro, Lichtenberg, Snow and Glacier](media/simulate.jpg)

### Surface

Surface nodes add the rock: strata, terraces, sandstone, shattered stone and
outcrops.

![Stratify, Sandstone, Shatter and Outcrops](media/surface.jpg)

### Colorize

Colour nodes paint the terrain from palettes, gradients and the masks the
other nodes make.

![The Badlands example as a heightfield, and coloured by CLUTer](media/colorize.jpg)

### Export

Build and Export writes the nodes marked for export as heightfields, masks and
colour maps (EXR, PNG, TIFF, JPEG or RAW), as one image each or as tiles.

![Height, flow, wear, deposits and colour maps from the Erosion Data Maps example](media/maps.jpg)

![Build Settings listing the nodes marked for export](media/build-settings.png)

### Examples

File › Open Examples… opens one of the 16 example projects Nerthus comes with.

![The Fractal Terraces example turning in the viewport](media/turntable.gif)

![The example projects](media/examples.jpg)

## Building

Nerthus needs Rust 1.99 or newer. It is developed on macOS; Windows and Linux
builds are untested.

```sh
cargo build --release
```

On macOS (Apple Silicon), the [Build workflow](.github/workflows/build.yml)
uploads `nerthus-macos-arm64.zip`, containing `Nerthus.app` with the
command-line builder, app icon and licences. Extract it and move the app to
Applications. The app is ad-hoc signed; it is not notarized with an Apple
developer certificate.

To package an existing release build locally on macOS:

```sh
brew install librsvg
packaging/package-macos.sh
```

The script takes optional binary and output directories, defaulting to
`target/release` and `target/packages`. It packages the existing binaries
without rebuilding them.

The builder in a macOS bundle is `Nerthus.app/Contents/MacOS/nerthus-build`.

## Running

The editor:

```sh
cargo run --release --bin nerthus -- path/to/project.terrain
```

`nerthus --help` (or `-?`) lists the editor's options, such as `--no-start`
to skip the Start dialog, `--safe-mode` to start with the default
preferences and options, and `--proxy <url>` to set the proxy used for
update checks.

Command-line builds:

```sh
cargo run --release --bin nerthus-build -- build path/to/project.terrain
```

Builds go to a new numbered folder under `Documents/Nerthus/Builds/<project
name>/` unless the project's Build Settings say otherwise. Each run writes a
log to `Logs` in the Nerthus user folder (`~/.nerthus`, or
`%LOCALAPPDATA%\Nerthus` on Windows); `--verbose` adds the debugging
messages, and `--safemode` builds with the default preferences instead of
the saved ones. Run `nerthus-build --help` for the other commands and
options.

For many builds in a row, `nerthus-build host` keeps a build host loaded
between them: the first run starts it, later runs hand their project and
options to it and print its output, and `nerthus-build host --exit` closes
it.

## Web version

The editor also runs in a browser with WebGL 2. The Web workflow builds it
for every pull request and publishes main's with GitHub Pages, at
<https://nerthus.app>.

To build it yourself, add the WebAssembly target and the `wasm-bindgen`
command of the version in `Cargo.lock`, then run the build script, which
writes the site to `target/web`:

```sh
rustup target add wasm32-unknown-unknown
cargo install wasm-bindgen-cli --version <version in Cargo.lock> --locked
crates/nerthus-app/web/build.sh
python3 -m http.server --directory target/web
```

A web page has no files, threads or other programs to use, so the web
version differs from the desktop one:

- Nodes build on the CPU alone and on the page's one thread, a node at a
  time between frames, so builds take longer.
- Projects are the examples it comes with and files you pick with Open;
  saving downloads the project. Settings, recent files and autosaves are
  not kept between visits.
- Build and Export, baking and the disk cache, screenshots and turntables,
  importing files, installing and exporting macros, loading and saving
  cameras, File nodes reading from disk, Diagnostics, update checks and the
  MCP server are left out.

## MCP server

The editor can run an [MCP](https://modelcontextprotocol.io) server, so AI
assistants such as Claude Code can work on the open project: add and connect
nodes, set parameters, build nodes and look at the results, and run Build and
Export. Their edits show in the editor as they happen and can be undone.

![A graph built over MCP: a Mountain, ErosionPro and SatelliteMap added, the Mountain's seed changed and the camera moved](media/mcp.gif)

Turn it on with Tools › MCP Server (it stays on for later starts), or start
the editor with `--mcp`. It listens on `http://127.0.0.1:47110/mcp`;
`--mcp-port <port>` picks another port. To connect Claude Code:

```sh
claude mcp add --transport http nerthus http://127.0.0.1:47110/mcp
```

The server only takes connections from this machine and has no other
authentication, so while it is on, any program running on this machine can
use it.

Its tools:

- **The project**: `get_project`, `open_project`, `new_project`,
  `save_project`, `set_project_settings`, `list_variables`, `set_variable`.
- **Node types**: `search_node_types`, `describe_node_type` (with how the
  example projects use each type and what `calibrate_nodes` measured it to
  make), `describe_modifier`, `suggest_nodes`. Types can also be named as
  `.terrain` files name them (`Erosion2` for ErosionPro, `SatMap` for
  SatelliteMap).
- **Building graphs**: `create_graph` (a whole graph of nodes, modifiers,
  export marks and connections as one undoable edit), `list_examples`,
  `open_example`, `add_node`, `set_parameters`, `update_node`,
  `connect_nodes`, `disconnect_input`, `delete_nodes`, `set_modifier`,
  `undo`, `redo`.
- **Checking graphs**: `list_nodes`, `get_node`, `select_node`,
  `validate_graph` (the open project, a file or a proposed graph; it can fix
  what is safe to fix and build it small to be sure), `analyze_graph`,
  `optimize_parameters`.
- **Building and looking**: `build_node`, `get_node_image`, `capture_view`,
  `set_camera`, `export_node_map`, `mark_for_export`, `build_and_export`,
  `run_project` (any saved project, with the command line's options:
  resolution, build folder, profile, region, mutation seed, variables, one
  node), `build_history`, `analyze_build`, `analyze_terrain` (relief in
  metres, gentle ground, edges against the middle, profiles),
  `calibrate_nodes`.
- **Project files**: `get_project_file`, `list_projects`,
  `repair_project_file`, `patch_project_file`, which change a `.terrain`
  file without opening it in the editor.

## Missing Features

These parts of Gaea 2.3.1.0 are missing from Nerthus or work differently in it.

### Variables

- Colour variables don't change the properties bound to them. Gaea applies a
  Color variable to the Color properties it is bound to (Constant,
  ColorThreshold, Tint, WaterColor and LightX's sun and ambient colours).
  Nerthus keeps the colour as a plain `R`, `G`, `B` object that its builds
  don't read, so a bound property keeps its own colour.
- Range variables made in Nerthus (by binding a range, or in the Vars panel)
  are saved with a type name Gaea doesn't have, so Gaea refuses to open a
  project that holds one.
- Variable values given as text use different formats for Range and Color
  variables. Gaea writes and reads a Range as `0.200 x 0.700` and a Color as
  `#RRGGBB` or `0.7000 0.2000 0.1000`; Nerthus uses `0.2, 0.7` and
  `0.7,0.2,0.1`. Build command lines, `-v`, `--va` and `--vars` values, and
  batch files written for Gaea therefore don't set those variables here (the
  comma in a Range also splits the `--va` list). Gaea also logs a variable
  it could not apply ("Variable [X] could not be applied to N's [P] property
  as the data [V] was not valid."); Nerthus skips it without a word.

### Math node

- Expressions can't use ExprTk's local vectors (`var v[3] := {1, 2, 3}`,
  `sum(v)`, `v[]`, arithmetic on vectors), strings, `return` or `assert`. Gaea
  builds them; Nerthus fails the node with an error.
- A bracket followed by another bracket (`(x)(y)`, `(a)[b]`) is not
  multiplied: Nerthus drops the first factor without an error.
- An unconnected `a`, `b` or `c` keeps what the expression last assigned to
  it for the whole image in Gaea (a running total in scan order); Nerthus
  starts every row again.
- `swap(x, y)` and `x <=> y` give the first variable's old value in Gaea and
  the second's in Nerthus when used as a value.
- Gaea fails an expression nested more than 400 deep and names ExprTk's
  lexer errors (`Invalid numeric token: 1.2`); Nerthus has no depth limit and
  one generic message.

### Builds and the command line

- A build folder that can't be created falls back to the default Builds
  folder in Gaea ("The build path could not be created. … Reverting to
  default path in BUILDS folder."); Nerthus refuses the build.
- `--output` with a destination template (`[Filename]`, `[+++]`) also
  creates a folder with the template's literal name before building.
- `--resolution` takes 512, 1024, 2048, 4096, 8192 or 16384 in Gaea and
  ignores any other number, so the project's own resolution is built;
  Nerthus builds at whatever number it is given.
- Starting a build in the editor (Project › Build and Export, its shortcut,
  or the Build button in Build Settings) deletes loose nodes in Gaea. Once
  "Build and Export?" is accepted and the project is saved, every node that
  has no connection at all, is not an Export node and is not marked for
  export is removed from the open project; the file just saved still holds
  it. Nerthus keeps them.
- Numbers in a `--vars` JSON file are read the way Gaea prints them (`5.0` is
  `5`, `1e5` is `100000`); Nerthus keeps `5.0`, so an Int or Choice variable
  set that way is ignored.
- Most simulations show no progress while they build. Gaea fills the footer
  progress bar and status text for Erosion, Wizard, Snow, Shrubs, Anastomosis,
  FlowMapClassic, Craggy, Uplift, Sandstone, MountainSide, Distress, Recurve,
  Stratify and Debris ("Rendering pass 1 / 3" and similar); Nerthus does for
  ErosionPro, EasyErosion, Wizard2, Sediments and Snowfield only.
- The Stones output of Stones and Scree is worked out from the stone surface
  before it is clamped to 0–1; Gaea clamps first, so the mask differs wherever
  stones push the terrain past 0 or 1.

### Nodes and properties

- New nodes don't get the port labels Gaea gives them: MathX's inputs read
  a, b and c, RGBMerge's R, G and B, TextureBaker's and MeshWarp's first input
  Mesher, Construction's In Base, Gabor's Modulation Direction and RGBSplit's
  Out Gray. Nerthus shows such a label only when a loaded file has it.
- The menus for dropping a connection on a node and for Insert chokepoint list
  ports by their raw names (`AreaMask`); Gaea writes `Area Mask`, and orders
  them as the node does.
- Reset (Reset Node, Ctrl+R, Reset to Defaults) also removes a node's
  modifiers in Gaea and leaves bypassed and baked nodes alone; Nerthus resets
  the parameters only. Reset Node on a Macro node puts its variables back to
  their defaults in Gaea; Nerthus replaces them with a placeholder variable.
- A TextureBaker or MeshWarp that is fed by a built Mesher marks itself for
  export when its panel shows, so its files are written in the default
  heightfield format; Nerthus needs Mark for Export.
- A Macro node's variables in the property panel have no colour editor, no
  group or separator rows and no ordering by the variable's order. A Macro
  node also never updates its ports and variables when the installed macro
  changes.
- Restrict to Group on Accumulator and WaterColor does nothing in Nerthus.
  Gaea's editor tells each node which group it sits in and the accumulation
  then counts only that group's nodes; Nerthus never passes the group to a
  build. Gaea's command-line builds don't either.
- Switch's Discard Alt in Final Build is ignored. In Gaea's command-line
  builds it disconnects the Alternate input, and the Switch then fails with
  "Required port [In] not connected."; Nerthus builds it as usual.
- Bomber hides Height, Variable Height and Proportional once it has built a
  colour map; Nerthus keeps showing them.
- Combine's Swap Inputs (with more than two inputs) and File's Purge Cache
  (with Never Cache on) stay visible in Gaea; Nerthus hides them.
- A new EasyErosion node starts with Seed 0 in Gaea and a new FractalTerraces
  node has two different seeds; Nerthus draws one random seed for both.
- SatelliteMap's Randomize picks a palette from the node's own library. Nerthus
  draws a number below the size of the Rock library (594) and clamps it to
  the library, so Blue and Color mostly land on their last palette.
- Nodes that can grow ports (Layers, Combine, Bomber, Construction, Mixer)
  sort their extra input and output rows by name in Gaea (`Layer1…Layer3`
  above `Mask1…Mask3`; `Input2` before `Input10`); Nerthus keeps the order the
  ports were added in.
- Option bars show a description as it is written, so PointCloud's Format
  bar reads `PLY (.ply)`, `PLY Binary (.ply)`, `XYZ (.xyz)` and `Unreal CSV
  (.csv)`; Nerthus title-cases them (`PLY (.Ply)`). Hovering a cell of the
  Export Format or Halftone Pattern grid shows the description (`Pure Raw
  (.graw)`, `Floyd Steinberg`) in Gaea and the raw member name (`GaeaRaw`,
  `FloydSteinberg`) in Nerthus.
- The first group of MathX, FlowMap and FlowMapClassic is headed with the class
  name in Gaea (MathX, FlowMap, FlowMapClassic); Nerthus heads it with the
  node's name (Math, Flow, FlowClassic).
- The per-property menu follows rules in Gaea: Expression… only for float
  properties, Bind to not for gradient lists, and neither for rows that can't
  be bound. Every row has the menu, Reset included. Nerthus offers
  Expression… on every property, and the curve, palette-list and gradient
  rows have no menu.

### Graph, menus and dialogs

- The Render Intent override menu in Gaea greys out Heightfield and Mask for
  colour nodes (SatelliteMap, CLUTer, HSL and the like), and choosing any
  intent for the node pinned as the underlay un-pins it. Nerthus offers every
  choice on every node and keeps the pin.
- Dragging a connection into empty space lists the node types that most often
  followed that port, strongest first. Nerthus re-sorts that list by how often
  each node type has been added at all.
- Options › Experimental › Custom Locations does nothing. Gaea adds those
  folders (and the Projects folder) to the Open, Save, Import Image, Install
  Macro and Export Macro dialogs, starts them in the Projects folder with
  Show only these locations, and refuses to keep a path that doesn't exist.
- A new, empty note shows "This is a Note, can be used to describe details"
  in Gaea; in Nerthus it is blank.
- Options › Build › Remove Primary port name in Build output does nothing in
  Nerthus. In Gaea it decides whether the same box in a new project's Build
  Settings starts ticked; Nerthus's new projects start unticked.
- Gaea greys out Node Commands and Node menu items the selection can't use:
  Select upstream/downstream and Convert port to portals unless exactly one
  node is selected, Combine nodes below two, Group / Ungroup and Move to Tab
  with nothing selected, Zoom › Selection and Presets without a selection,
  Lock Preview and Locked 2D Viewport with several selected, and a locked
  group's colour and icon. Nerthus leaves them enabled (Group / Ungroup with
  nothing selected makes an empty group).
- Select All (Ctrl+A) selects every node, group, note and connection of every
  graph tab in Gaea, and deleting a selection that spans tabs asks "Selected
  elements contains elements from other graphs. Do you want to continue
  deleting selected items?"; Nerthus selects the current tab's nodes only.
- Dropping a connection on the middle of a node shows that node's ports menu
  in Gaea: its ports on the side the connection reaches (the primary port
  included), Add as Mask on a node that can take a mask and Add new port on
  one that can grow; the outer fifths of the node just drop the connection.
  Nerthus shows that menu only when the connection lands on a compact node's
  collapsed port, and without the primary port. A drop anywhere else on a
  node opens the node search, so an ordinary node can't be given a mask or an
  extra port by dropping on it, which Gaea's own docs describe.
- Right-clicking empty graph clears the selection and opens the node search
  in Gaea, and only the middle button pans. Nerthus opens a menu of node
  categories, Add Note and Zoom to Extents instead, and also pans on a
  right-button drag.
- Connecting two nodes selects the node the wire ends at in Gaea, so the
  property panel and the preview switch to it; Nerthus leaves the selection
  where it was.
- Pressing on a connected input and dragging starts a new connection from that
  input in Gaea, and the existing wire stays until a new one replaces it.
  Nerthus unplugs the wire at once and carries its source end, so a slip
  removes a connection and letting go on empty graph offers nodes that feed
  from the old source.
- A plain click on one of several selected nodes, groups or notes keeps the
  whole selection in Gaea, so Delete, Bypass and Group still act on all of
  them; Nerthus reduces the selection to the clicked element.
- Bypass (B, the Node menu) does nothing to a baked node in Gaea; in Nerthus
  it replaces the bake with a bypass.
- Gaea selects several notes at once (box select, Ctrl or Shift click), and
  moving, deleting, grouping and Move to Tab act on all of them; Nerthus
  selects one note at a time.
- Dragging a group or a note moves the whole selection in Gaea, and Snap to
  Grid and Snap to Items apply to it. In Nerthus a group (with its own nodes
  and notes) or a note moves alone and never snaps.
- Zoom differs. Reset Zoom (Ctrl+1) sets 100% and moves the view to the top
  left of the nodes in Gaea; Nerthus zooms to 100% about the middle of the
  view. Gaea draws nodes in their simplified form below 50% zoom (Nerthus
  below 40%), multiplies the zoom by 1.26 per wheel notch (Nerthus's step is
  smaller), and has no upper limit, where Nerthus stops at 200% and opens a
  tab that Gaea saved at a larger zoom at 200%.
- A port's dot is drawn at full strength once something is connected to it and
  dimmed when nothing is. Gaea does this for outputs too; Nerthus dims every
  output, wired or not, so the snap outline (red on a connected port, blue on
  a free one) is always blue on an output. The "all ports" dot of a compact or
  small node doesn't light up in Nerthus when a hidden port is wired, as it
  does in Gaea.
- The node used as the underlay carries a small magenta icon in its icon stack
  in Gaea; Nerthus marks it nowhere on the graph.
- Holding Shift while choosing Bake › Cache to Disk › Delete Cache writes
  every node's maps out to the disk cache and frees them in Gaea, instead of
  deleting the cache; Nerthus has no such gesture. Without Shift, Gaea's
  Delete Cache also marks the project modified; Nerthus's doesn't.
- Saving warns about unbound variables in Gaea: every save (Ctrl+S, Save As,
  Save Incremental, Export Macro, Build and Export, baking) first shows
  "Orphan Variables Detected" with the variables that have no binding, unless
  the Options box is off. Nerthus shows it only after a mass delete.
- Clicking GAEA in the title bar opens About in Gaea; Nerthus's brand is
  plain text.
- Gaea shows these outcomes as a coloured alert bar over the graph that
  dismisses itself: Repair File's result, Delete Cache's, Clear Cache's and
  Clean Memory's, and Bake Selected's "Please wait…", "Success!" and "Error!"
  (and a baked cache that fails to load). Nerthus puts them in the status bar
  only, where the next message replaces them.
- The status bar reads Idle, Working or Suspended in Gaea; Nerthus reads Idle,
  Building, Engine suspended or Cancelling.
- Small differences: an unsaved project is `Untitled` in Gaea and
  `Untitled.terrain` in Nerthus; the recent-file icons number each entry by
  its place in the list, so a deleted file leaves a gap; the file box flashes
  green after a save and when the engine resumes; Export Macro on an unsaved
  project asks where to save first, and baking one saves it and carries on;
  the floating viewport window stays on top of the editor while the editor is
  active; a note added from the Graph menu lands near the top left of the
  view and moves right of whatever is already there, where Nerthus puts it in
  the middle of the view.
- Mutate (the Node menu and Mutate all nodes) is refused while a build is
  running ("Did not mutate. An existing build is in progress."); Nerthus
  re-seeds the nodes and restarts the build.
- Deleting the only graph tab shows "You must have at least one graph tab in
  your project. You cannot delete this tab until you create another one."
  in Gaea; Nerthus greys the item out.
- Build Settings › Copy Command Line opens the Build Command Line window in
  Gaea (the command, Copy to Clipboard, Close) instead of copying at once.
  The Terrain tab's readout reads `Real scale 2.441m/px` and `Height-Scale
  Ratio: 00.50` in a monospaced font; Nerthus words and formats it
  differently.
- The Undo menu words several edits differently: "New Note added", "{name}
  moved", "Group renamed: {old} renamed to {new}.", "Graph tab ({name})
  renamed", "Graph color changed.", "Property {name} bound to new variable",
  "Variable for property {name} unbound" and, for a node added from the
  toolbox or search, "Node added: {name}".

### Viewport

- Options › Viewport doesn't preview the background colour. Gaea changes the
  3D view's background while the colour is being picked and puts it back when
  Options is cancelled; Nerthus changes it when Options is accepted. Gaea
  also keeps the colour as linear values where Nerthus keeps sRGB, so one
  options file shows different backgrounds in the two.
- Recording a turntable gives less feedback and different frames. Gaea shows
  "Frame captured: <file>" in the status bar for each frame and a busy cursor
  until the recording ends, and its frames include the compass unless Hide
  Grid and Compass is ticked; Nerthus's never include the compass.
- Reset Sky also sets Mask Gradient to Red in Nerthus. Gaea resets the sun,
  air, haze, ozone, fog, ambient, ground, shadow, skybox and exposure
  settings only, so a mask preview keeps the gradient chosen in Options or in
  the panel.
- With Show Skybox on, Gaea takes the fog colour from a 256-step gradient
  picture of its own, indexed by the sun's elevation (orange at the horizon,
  blue high up). Nerthus works the colour out from its own sky, so the haze
  over the terrain is paler.

## Intentional differences

These features of Gaea are intentionally left out of Nerthus, or are implemented differently:

- Licensing.
- The Houdini, Unity and Unreal bridges: the HoudiniOut, Unity and Unreal
  nodes, the File and Export nodes' Prepare for Bridge buttons, Create
  Connected Nodes › For Plugins, and Tools › Install Plugins.
- Features Gaea ships disabled or unfinished, such as Network Build.
- Drawing Windows metafiles (`.emf`) exactly as Windows does. Nerthus draws
  them itself, so edges and text differ slightly, and fonts that aren't
  installed are replaced by similar ones.
- On Windows, a console window opens beside the editor every time it
  starts. Gaea opens one only when it's started with `-Console`.
- Reading EPS, PostScript and PDF files. Gaea hands them to Ghostscript, a
  separately installed program that draws them with its own fonts.
- Camera raw files from makers outside the eight families the port reads.
  Nerthus's camera raw support covers Canon, Nikon, Sony, Fujifilm,
  Olympus, Panasonic, Pentax and Samsung; Gaea also opens every other
  maker LibRaw supports (Leica, Sigma, Kodak, Minolta, GoPro, DJI and the
  rest). Within the eight families, three things Gaea can read stayed out
  on purpose: Sony's R1, whose sensor data is YCC; Nikon's Coolscan film
  scanners; and Panasonic's encoding-8 RW2 streams. DNG files from any
  maker are unaffected.
- Gaea's support services: Watson's support ticket form, and its upload of
  crash logs to QuadSpinner in return for a Log Receipt code.
- Gaea's interface fonts. Nerthus uses Roboto and Roboto Mono, so text is
  spaced a little differently and monospaced text is about a fifth wider.
- The icons that are Gaea's own or licensed artwork, about 100 of them
  (45 node icons, 13 family and toolbox icons, and interface icons).
  Nerthus shows Tabler icons in their place.
- Gaea's overlay images and example projects. Nerthus draws its own overlay
  patterns and ships 16 examples: 12 made for Nerthus and 4 contributed
  under the MIT licence.
- Gaea's colour library. Nerthus's palettes are its own, taken from
  Sentinel-2 satellite imagery to look like Gaea's palette with the same
  number, so SatelliteMap, SuperColor, Cartography through a SatelliteMap,
  and Weathering build with somewhat different colours from Gaea's, and
  TextureBase with New accentuation builds a somewhat different mask
  (Weathering and TextureBase read Rock palette 117). Projects saved by Gaea
  keep their palette numbers.

## License

MIT (see [LICENSE](LICENSE)). The Roboto and Roboto Mono fonts in
`crates/nerthus-app/assets/fonts` are under the Apache License 2.0, with their
licence texts beside them. The palettes in `crates/nerthus-nodes/data/color`
contain modified Copernicus Sentinel data 2023–2024; the
[README](crates/nerthus-nodes/data/color/README.md) beside them says where
each one comes from.

The documentation in `crates/nerthus-app/assets/docs` (Help › User Guide) is
adapted from Gaea 2's documentation, which is published under the MIT licence;
its licence text is in `crates/nerthus-app/assets/docs/LICENSE.txt`.

Four of the example projects in `crates/nerthus-app/assets/examples`
(Distance Ridges, Fractal Terraces, Kyrgyzstan Valley and Vosges Geology) were
contributed to Nerthus under the MIT licence.
