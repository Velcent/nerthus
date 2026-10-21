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
- Numbers in a `--vars` JSON file are read the way Gaea prints them (`5.0` is
  `5`, `1e5` is `100000`); Nerthus keeps `5.0`, so an Int or Choice variable
  set that way is ignored.
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
