//! What the property area shows for a node: the `Node.UpdateUI`
//! overrides, which fill `VisibilityTable` and `EnableTable` from the
//! node's values and connections, and write the text some nodes display
//! in their computed-string properties (`LODInfo`, `TopologyWarning`, …).
//!
//! TileInput's computed summary is filled by its tile discovery build;
//! the baker file lists also read the build destination and cached inputs.
//! The skipped hook nodes (Unity, Unreal, HoudiniOut) have no UI here.

use std::collections::HashMap;
use std::path::Path;

use indexmap::IndexMap;
use nerthus_engine::automation::Automation;
use nerthus_engine::{Catalogue, NodeData, NodeSpec, NodeState};
use serde_json::Value;

use crate::editor::Editor;

/// The panel state `UpdateUI` leaves for one node.
#[derive(Default)]
pub struct NodeUi {
    visibility: HashMap<String, bool>,
    enabled: HashMap<String, bool>,
    /// Values of computed-string properties.
    pub texts: HashMap<String, String>,
}

impl NodeUi {
    /// Whether the property's editor shows (properties the tables do not
    /// mention show).
    pub fn visible(&self, name: &str) -> bool {
        self.visibility.get(name).copied().unwrap_or(true)
    }

    pub fn enabled(&self, name: &str) -> bool {
        self.enabled.get(name).copied().unwrap_or(true)
    }

    fn show(&mut self, name: &str, visible: bool) {
        self.visibility.insert(name.to_string(), visible);
    }

    fn show_all(&mut self, names: &[&str], visible: bool) {
        for name in names {
            self.show(name, visible);
        }
    }

    fn enable(&mut self, name: &str, enabled: bool) {
        self.enabled.insert(name.to_string(), enabled);
    }

    fn enable_all(&mut self, names: &[&str], enabled: bool) {
        for name in names {
            self.enable(name, enabled);
        }
    }
}

/// The computed file list needs the same destination and overwrite mode
/// as a build, and is empty until the Mesher input has a built map.
pub fn update_file_summaries(editor: &Editor, node: &NodeData, ui: &mut NodeUi) {
    if node.type_name == "TileInput" && editor.status(node.id).state == NodeState::Error {
        ui.texts.insert("TileInfo".to_string(), String::new());
        return;
    }
    if !matches!(node.type_name.as_str(), "TextureBaker" | "MeshWarp") {
        return;
    }
    ui.texts.insert("LODInfo".to_string(), String::new());
    let Some(input) = node.port("In").and_then(|port| port.record.as_ref()) else {
        return;
    };
    let asset = &editor.document.project.assets[0];
    let mesher_connected = asset.terrain.nodes.get(&input.from).is_some_and(|source| source.type_name == "Mesher");
    if !mesher_connected || editor.status(input.from).state == NodeState::Error {
        return;
    }
    let has_map = editor.node_caches.get(&input.from).is_some_and(|cache| cache.ports.iter().any(|port| port.name == input.from_port));
    if !has_map {
        return;
    }
    let mut definition = nerthus_export::BuildDefinition::from_json(asset.extra.get("BuildDefinition"));
    let region = nerthus_export::build::set_up_region(&asset.terrain, &mut definition, None);
    let single_image = definition.output_type == nerthus_export::build::OutputType::SingleImage;
    let region_build = definition.build_type == nerthus_export::build::BuildType::Region;
    if !single_image && !region_build {
        return;
    }
    let preferences = editor.user.preferences.export_preferences();
    let project_path = editor.document.path.as_deref().unwrap_or_else(|| Path::new("Untitled.terrain"));
    let mut destination = nerthus_export::destination::DestinationContext::new(&definition.destination, project_path, definition.resolution);
    destination.builds_root = preferences.builds_root;
    destination.region = region.map(|region| region.name);
    destination.mutation = editor.mutation;
    let folder = nerthus_export::destination::resolve_destination(&destination);
    let groups = nerthus_export::mesh_exports::texture_bake_files(&asset.terrain, node, &folder, definition.overwrite_mode);
    let mut info = String::from("Files to be generated:\n\n");
    for files in groups {
        for file in files {
            info.push_str(&file.to_string_lossy());
            info.push('\n');
        }
        info.push('\n');
    }
    ui.texts.insert("LODInfo".to_string(), info);
}

/// A node's values and connections as `UpdateUI` reads them.
struct Props<'a> {
    node: &'a NodeData,
    spec: Option<&'static NodeSpec>,
    nodes: &'a IndexMap<i32, NodeData>,
}

impl Props<'_> {
    fn boolean(&self, name: &str) -> bool {
        self.node.values.get(name).and_then(Value::as_bool).unwrap_or(false)
    }

    fn int(&self, name: &str) -> i64 {
        self.node.values.get(name).and_then(Value::as_i64).unwrap_or(0)
    }

    fn float(&self, name: &str) -> f32 {
        self.node.values.get(name).and_then(Value::as_f64).unwrap_or(0.0) as f32
    }

    /// An enum property's member name.
    fn member(&self, name: &str) -> &str {
        self.node.values.get(name).and_then(Value::as_str).unwrap_or_default()
    }

    fn is(&self, name: &str, member: &str) -> bool {
        self.member(name) == member
    }

    /// An enum property's integer value.
    fn enum_value(&self, name: &str) -> i64 {
        let parameter = self.spec.and_then(|spec| spec.parameter(name));
        parameter.and_then(|parameter| parameter.enum_value(self.member(name))).unwrap_or(0)
    }

    /// `(uint)(value - low) <= (uint)(high - low)`: the value is in
    /// `low..=high`.
    fn enum_in(&self, name: &str, low: i64, high: i64) -> bool {
        (low..=high).contains(&self.enum_value(name))
    }

    /// `Node.Ins`: the input ports in order.
    fn inputs(&self) -> Vec<&nerthus_engine::PortData> {
        self.node.ports.iter().filter(|port| port.is_input()).collect()
    }

    fn input_connected(&self, index: usize) -> bool {
        self.inputs().get(index).is_some_and(|port| port.record.is_some())
    }

    fn port_connected(&self, name: &str) -> bool {
        self.node.port(name).is_some_and(|port| port.record.is_some())
    }

    /// The type of the node feeding an input.
    fn source_type(&self, port: &str) -> Option<&str> {
        let record = self.node.port(port)?.record.as_ref()?;
        let source = self.nodes.get(&record.from)?;
        Some(source.type_name.as_str())
    }
}

/// .NET's `{n:###,###,###}`: digits grouped by commas, nothing for zero.
fn grouped(value: i64) -> String {
    if value == 0 {
        return String::new();
    }
    let digits = value.unsigned_abs().to_string();
    let mut text = String::new();
    for (index, digit) in digits.chars().enumerate() {
        let remaining = digits.len() - index;
        if index > 0 && remaining.is_multiple_of(3) {
            text.push(',');
        }
        text.push(digit);
    }
    if value < 0 {
        text.insert(0, '-');
    }
    text
}

/// `Node.UpdateUI` for `node`.
pub fn node_ui(node: &NodeData, nodes: &IndexMap<i32, NodeData>, automation: &Automation) -> NodeUi {
    let props = Props {
        node,
        spec: Catalogue::get().node(&node.type_name),
        nodes,
    };
    let mut ui = NodeUi::default();
    let bound = |property: &str| automation.bindings.iter().any(|binding| binding.node == node.id && binding.property == property);

    match node.type_name.as_str() {
        "Pattern" => {
            let dots = props.enum_in("Type", 2, 3);
            ui.show_all(&["DotSize", "Spacing"], dots);
            ui.show_all(&["Steps", "Size", "PixelSize", "Direction"], !dots);
        }
        "Snow" => {
            let melt_map = props.port_connected("MeltMap");
            ui.show_all(&["TerrainScale", "Verticality"], !props.boolean("RealScale"));
            ui.show("MeltType", !melt_map);
            let directional = !melt_map && props.is("MeltType", "Directional");
            ui.show_all(&["MeltRemnants", "Direction"], directional);
        }
        "SBomber" => ui.show("Height", props.boolean("VariableHeight")),
        "MultiFractal" => {
            ui.show("Octaves", !props.boolean("AutoOctaves"));
            ui.show("EdgeSmoothing", props.enum_in("NoiseType", 1, 2));
            ui.show("Bias", props.is("VariationType", "SecondaryFractal"));
            let warped = props.enum_in("Perturb", 1, 2);
            ui.show_all(&["RelativeWarpSize", "RelativeWarpAnisotropy", "WarpStrength", "WarpAttenuation", "WarpIterations"], warped);
            ui.show_all(&["WarpComplexity", "WarpRoughness"], props.is("Perturb", "Complex"));
        }
        "MountainSide" => ui.show_all(&["Detail", "Direction"], props.is("Type", "Slope")),
        "Snowfield" => {
            ui.show("FlowsDepth", props.boolean("Flows"));
            ui.show("TaperAmount", props.boolean("TaperSnowfall"));
        }
        "Mesher" => mesher(&props, &mut ui),
        "Glacier" => {
            ui.show_all(&["DiagonalBreakageDirection", "BreakageCount"], props.boolean("DiagonalBreakage"));
            ui.show_all(&["FlowBreakageDepth", "Extreme"], props.boolean("FlowBreakage"));
            ui.show_all(&["SubstructureDensity", "SubstructureDepth"], props.boolean("Substructure"));
        }
        "Crater" => {
            let new_style = props.is("Style", "New");
            ui.show_all(&["Shape", "Rim"], !new_style);
            ui.show_all(&["X", "Y", "Seed"], new_style);
        }
        "Slope" => ui.show("MicroAccent", props.is("Type", "Accurate")),
        "Shrubs" => ui.enable_all(&["CustomInfluence", "Trim"], props.input_connected(1)),
        "Sea" => ui.show_all(&["ShoreSize", "ShoreHeight", "Variation", "ExtraCliffDetails"], props.boolean("CoastalErosion")),
        "LoopEnd" => ui.show("Seed", props.boolean("MutateSeeds")),
        "Export" => {
            let path_bound = bound("OutputPath");
            ui.show_all(&["HoudiniOut", "InformationHoudini"], !path_bound);
            ui.show("OutputPath", !props.is("Location", "BuildFolder"));
        }
        "ErosionPro" => {
            let enabled = props.boolean("Enable");
            ui.show_all(&["Reverse", "Altitude", "Slope", "RainShadow", "Direction", "DirectionalPrecipitation"], enabled);
            let directional = enabled && props.boolean("DirectionalPrecipitation");
            ui.show_all(&["Direction", "RainShadow"], directional);
            let erodes = !props.boolean("NoErosion");
            ui.enable_all(&["CoarseSedimentsDischargeAngle", "CoarseSedimentsDischargeAmount", "BedLoadDischargeAngle", "BedLoadDischargeAmount", "Downcutting"], erodes);
            ui.enable_all(&["ShapeDetailScale", "ShapeSharpness", "Shape"], erodes);
        }
        "Distance" => {
            let classic = props.is("Method", "Classic");
            ui.show_all(&["FalloffJitter", "Angle", "Skew", "Directions", "Mode", "AngularJitter", "Seed"], !classic);
            ui.show_all(&["Threshold", "MultiplyByInput"], classic);
        }
        "Shaper" => {
            ui.show("LocalArea", props.boolean("LocalEffect"));
            ui.show("DetailSize", props.boolean("MaintainFineDetails"));
        }
        "Edge" => {
            ui.show("Pixels", props.is("Style", "Precise"));
            ui.show("Size", !props.is("Style", "Precise"));
            ui.show("Softness", props.is("Style", "Round"));
        }
        "Combine" => ui.show("SwitchInputs", props.int("PortCount") == 2),
        "Thermal" => ui.show_all(&["TerrainScale", "Verticality"], !props.boolean("RealScale")),
        "DataExtractor" => {
            ui.show("Level", props.is("Data", "Boundary"));
            ui.show("Layer", props.is("Data", "MixerMask"));
        }
        "Sediments" => {
            ui.show("Angle", props.is("Type", "Fine"));
            ui.show("Passes", !props.is("Type", "Classic"));
            ui.show_all(&["Style", "GrainyDeposits"], props.is("Type", "Classic"));
        }
        "Layers" => {
            let layers = props.inputs().len() as i64 / 2 + 1;
            for index in 2..=8 {
                ui.show(&format!("Blend{index}"), layers >= index);
            }
        }
        "Constant" => {
            ui.show("Height", props.is("Output", "Height"));
            ui.show("Color", props.is("Output", "Color"));
        }
        "Pixelate" => ui.show("Size", !props.boolean("Autosize")),
        "Normals" => ui.show("DetailSize", !props.is("Type", "Standard")),
        "Compare" => ui.show_all(&["Perpendicular", "Colorized"], !props.is("Mode", "Difference")),
        "Gamma" => ui.show("GammaValue", !props.boolean("Automatic")),
        "Bomber" => {
            // `isRGB` is only known after a build; until then it is false,
            // and its last assignment wins over VariableHeight's.
            let is_rgb = false;
            ui.show("Height", props.boolean("VariableHeight"));
            ui.show("Jitter", props.is("Distribution", "Grid"));
            ui.show_all(&["Height", "VariableHeight", "MaintainProportions"], !is_rgb);
        }
        "Dusting" => {
            let on_snow = matches!(props.source_type("In"), Some("Snow" | "Snowfield"));
            ui.show("IncludeExistingSnow", on_snow);
            ui.show("Direction", props.boolean("Directional"));
        }
        "Aperture" => {
            ui.show("VerticesCount", props.enum_in("Shape", 1, 2));
            ui.show("Direction", props.enum_in("Shape", 1, 4));
            ui.show("CornerAngle", props.is("Shape", "Corner"));
        }
        "Erosion" => {
            ui.show_all(&["TerrainScale", "Verticality"], !props.boolean("RealScale"));
            let area_effect = !props.input_connected(1) && !props.is("AreaEffect", "None");
            ui.show_all(&["Reverse", "BiasType", "Bias"], area_effect);
        }
        "Accumulator" => {
            let warning = if props.port_connected("In") {
                String::new()
            } else {
                "⚠\u{fe0f} The Accumulator is not connected to any node. Please connect the Accumulator to the last node of the type you will use for masking.\n\nOtherwise it will show outdated data during build.".to_string()
            };
            ui.texts.insert("Warning".to_string(), warning);
        }
        "BlobRemover" => ui.show("Quantization", props.boolean("EnforceQuantization")),
        "Lake" => {
            let advanced = props.is("Type", "Advanced");
            ui.show_all(&["X", "Y", "WaterLevel", "Accuracy"], !advanced);
            ui.show_all(&["Precipitation", "SmallLakes", "FloodControl", "ShoreSize", "AltitudeBias", "SizeBias"], advanced);
            ui.show("WaterFloor", advanced && props.boolean("FloodControl"));
        }
        "Repeat" => ui.show("Height", !props.boolean("CompensateHeight")),
        "WaterColor" => ui.show_all(&["Shallow", "Deep"], props.is("Palette", "Custom")),
        "File" => {
            let name_bound = bound("FileName");
            ui.show_all(&["HoudiniIn", "InformationHoudini"], !name_bound);
            ui.show("PurgeCache", !props.boolean("NeverCache"));
        }
        "Trees" => trees(&props, &mut ui),
        "Mountain" => ui.show("ReduceDetails", props.enum_in("Style", 1, 2)),
        "Transpose" => {
            ui.show("Extend", props.is("Mode", "Embed"));
            ui.show_all(&["Threshold", "Flatten", "Boundary"], props.is("Mode", "Insert"));
        }
        "Shade" => {
            let table_top = props.is("RenderStyle", "TableTop");
            ui.show_all(&["Sunlight", "SoftShadows", "ReflectedLight"], table_top);
            ui.show("HighQuality", table_top || props.is("RenderStyle", "Occlusion"));
            let lit = table_top || props.is("RenderStyle", "SoftLight");
            ui.show_all(&["SunElevation", "SunAzimuth"], lit);
            ui.show("ShadeBias", !table_top);
        }
        "Debris" => ui.show("Information", props.boolean("ExportPointCloud")),
        "Sandstone" => ui.show("Chipping", props.boolean("Chipped")),
        "EasyErosion" => ui.show("Direction", props.is("Style", "Flows2") || props.is("Style", "Strata")),
        "Transform" => ui.show_all(&["ScaleX", "ScaleY"], !props.boolean("Uniform")),
        "LightX" => {
            let geographic = props.is("DirectionSelection", "Geographic");
            let direct = props.boolean("DirectLight");
            let ambient = props.boolean("AmbientLight");
            let automatic_colors = props.boolean("AutomaticColors");
            ui.show_all(&["North", "Latitude", "DayOfYear", "SolarHour"], direct && geographic);
            ui.show_all(&["SunAzimuth", "SunElevation", "Shadows"], direct && !geographic);
            ui.show("AngularDiameter", direct && props.boolean("Shadows"));
            ui.show("AmbientOcclusion", ambient);
            ui.show("SunColor", !automatic_colors && direct);
            ui.show("AmbientColor", !automatic_colors && ambient);
            ui.show("GroundBrightness", !props.input_connected(1));
            ui.show("SunIntensity", direct);
            ui.show("AmbientIntensity", ambient);
        }
        "HueErosion" => ui.enable("LaminarFlow", props.float("Diffusion") == 0.0),
        "GroundTexture" => ui.show_all(&["Coverage", "Density"], props.is("Method", "Harsh")),
        "Dilate" => ui.show("Shape", props.is("Kernel", "Rectangle")),
        "PointCloud" => ui.show("Scale", !props.is("Format", "XYZ")),
        "Median" => {
            ui.show_all(&["Horizontal", "Vertical"], props.is("Mode", "Advanced"));
            ui.show("Strength", !props.is("Mode", "Advanced"));
            ui.show("Quality", props.is("Mode", "Advanced2"));
        }
        "FractalTerraces" => {
            ui.show_all(&["TiltAmount", "Direction", "TiltSeed", "WarpAmount", "WarpSize", "WarpStyle"], props.boolean("ApplyTilt"));
            ui.show_all(&["MacroOctaves", "MicroShape"], props.boolean("ShapesSeparation"));
        }
        "MacroPort" => ui.show("Fallback", props.boolean("IsOptional")),
        "Filter" => {
            ui.show("Gain", props.enum_in("Mode", 2, 3) || props.is("Mode", "Bell"));
            ui.show("Q", props.enum_in("Mode", 4, 6));
        }
        "MathX" => ui.show("Expression", false),
        "Noise" => {
            let fixed = props.is("NoiseType", "Fixed");
            ui.show("Count", fixed);
            ui.show("Density", props.enum_in("NoiseType", 3, 4));
            ui.show("RelativeDensity", fixed);
        }
        "Halftone" => ui.show("Depth", props.boolean("Posterize")),
        "Shape" => {
            ui.show("Sides", false);
            ui.show_all(&["ScaleX", "ScaleY"], !props.boolean("Uniform"));
        }
        "Warp" => {
            ui.show("Perturbation", !props.is("NoiseType", "PerlinFBM"));
            ui.show_all(&["ModulationDirection", "Modulation"], props.input_connected(1));
        }
        _ => {}
    }
    ui
}

/// `Mesher.UpdateUI`: the topology warning and the mesh sizes per LOD.
fn mesher(props: &Props, ui: &mut NodeUi) {
    ui.show("TopologyWarning", false);
    let warning = match props.member("Topology") {
        "AdaptiveTris" => Some("Adaptive Tris may result in fewer vertices, as regions that become highly optimized may not require as many vertices."),
        "Quads" => Some("Quads are only supported for OBJ, FBX and DAE export. GLTF and GLB formats don't support quads."),
        _ => None,
    };
    if let Some(warning) = warning {
        ui.show("TopologyWarning", true);
        ui.texts.insert("TopologyWarning".to_string(), warning.to_string());
    }

    let mesh_line = |side: i64| {
        let vertices = grouped(side * side);
        let faces = grouped((side - 1) * (side - 1) * 2);
        format!("{side}x{side} | V: {vertices} | F: {faces}\n")
    };
    let levels = props.enum_value("LevelOfDetail");
    let mut side = props.int("VerticesPerSide") + 1;
    let info = if levels > 0 {
        let mut text = String::new();
        for level in 0..=levels {
            let name = if level == 0 { "Base Mesh".to_string() } else { format!("LOD{level}") };
            text.push_str(&format!("{name}\n{}\n", mesh_line(side)));
            side = side / 2 + 1;
        }
        text
    } else {
        format!("Base Mesh:\n{}", mesh_line(side))
    };
    ui.texts.insert("LODInfo".to_string(), info);
}

/// `Trees.UpdateUI`: instance settings and the tree count estimate.
fn trees(props: &Props, ui: &mut NodeUi) {
    let custom = props.input_connected(1);
    ui.enable_all(&["CustomInfluence", "Trim"], custom);
    ui.show("Trim", custom);
    let instances = props.is("Output", "Instances");
    ui.show_all(&["TreeSize", "TrimUnder", "Shape", "TreeCount", "LODInfo", "IsRelative", "Random", "Strict"], instances);
    if !instances {
        return;
    }
    let count = props.int("TreeCount");
    let info = if props.boolean("IsRelative") {
        let coverage = count as f32 * 1000.0 / 262144.0 * 100.0;
        let mut text = format!("Relative Tree Count:\nCoverage: ~{coverage:.1}%\n");
        for (resolution, factor) in [(512, 1000), (1024, 2000), (2048, 4000), (4096, 8000), (8192, 16000)] {
            text.push_str(&format!("@ {resolution}: {}\n", grouped(count * factor)));
        }
        text
    } else {
        format!("~{} trees will be generated.", grouped(count * 1000))
    };
    ui.texts.insert("LODInfo".to_string(), info);
}

#[cfg(test)]
mod tests {
    use crate::document::Document;
    use crate::worker::{CachedPort, NodeCache};
    use nerthus_engine::RenderIntent;
    use serde_json::json;

    use super::*;

    #[test]
    fn grouping_matches_dotnet() {
        assert_eq!(grouped(0), "");
        assert_eq!(grouped(7), "7");
        assert_eq!(grouped(1025), "1,025");
        assert_eq!(grouped(1_050_625), "1,050,625");
    }

    #[test]
    fn erosion_pro_hides_inhibitors_until_enabled() {
        let spec = Catalogue::get().node("ErosionPro").unwrap();
        let mut node = NodeData::new(spec, 1, 1);
        let nodes = IndexMap::new();
        let automation = Automation::default();
        let ui = node_ui(&node, &nodes, &automation);
        assert!(!ui.visible("Direction"));
        assert!(!ui.visible("Slope"));
        assert!(ui.visible("Duration"));
        node.values.insert("Enable".to_string(), Value::Bool(true));
        node.values.insert("NoErosion".to_string(), Value::Bool(true));
        let ui = node_ui(&node, &nodes, &automation);
        assert!(ui.visible("Slope"));
        assert!(!ui.visible("Direction"));
        assert!(!ui.enabled("Downcutting"));
    }

    #[test]
    fn baker_file_summaries_require_a_built_mesher_input() {
        let folder = std::env::temp_dir().join("nerthus-file-summary-preview");
        for type_name in ["TextureBaker", "MeshWarp"] {
            let mut document = Document::new();
            let mesher = document.add_node("Mesher", [0.0, 0.0]).unwrap();
            let baker = document.add_node(type_name, [100.0, 0.0]).unwrap();
            document.connect(mesher, "Out", baker, "In");
            document.rename(baker, "Baked");
            document.set_value(mesher, "LevelOfDetail", json!("LOD1"));
            document.set_member(baker, "SaveDefinition", json!({"Format": "PNG16"}), "Export format", false);
            document.set_build_setting("Destination", json!(folder));
            let mut editor = Editor::new(document);
            let node = editor.document.node(baker).unwrap().clone();
            let mut ui = NodeUi::default();
            update_file_summaries(&editor, &node, &mut ui);
            assert_eq!(ui.texts["LODInfo"], "");

            let port = CachedPort { name: "Out".to_string(), width: 4, height: 4, channels: 1, intent: RenderIntent::Heightfield };
            let cache = NodeCache { ports: vec![port], ..NodeCache::default() };
            editor.node_caches.insert(mesher, cache);
            update_file_summaries(&editor, &node, &mut ui);
            let expected = format!(
                "Files to be generated:\n\n{}\n{}\n\n",
                folder.join("Baked_Normals.png").display(),
                folder.join("Baked_Normals_LOD1.png").display(),
            );
            assert_eq!(ui.texts["LODInfo"], expected);

            let mut disconnected = node.clone();
            disconnected.port_mut("In").unwrap().record = None;
            update_file_summaries(&editor, &disconnected, &mut ui);
            assert_eq!(ui.texts["LODInfo"], "");

            let source = editor.document.project.terrain_mut().nodes.get_mut(&mesher).unwrap();
            source.type_name = "Constant".to_string();
            update_file_summaries(&editor, &node, &mut ui);
            assert_eq!(ui.texts["LODInfo"], "");
        }
    }

    #[test]
    fn failed_tile_builds_clear_the_displayed_summary() {
        let mut document = Document::new();
        let tile_input = document.add_node("TileInput", [0.0, 0.0]).unwrap();
        document.apply_build_update(tile_input, "TileInfo", json!("Total tiles: 4"));
        let mut editor = Editor::new(document);
        editor.statuses.entry(tile_input).or_default().state = NodeState::Error;
        let node = editor.document.node(tile_input).unwrap();
        let mut ui = NodeUi::default();
        update_file_summaries(&editor, node, &mut ui);
        assert_eq!(ui.texts["TileInfo"], "");
    }
}
