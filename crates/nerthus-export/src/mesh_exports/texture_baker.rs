//! TextureBaker's and MeshWarp's exports (the two nodes are the same code;
//! MeshWarp is hidden). For each level of detail of the Mesher
//! feeding `In`, the Mesher's map is resized to that level's vertex count
//! and made a grid mesh, then baked against the full map: the primary
//! port writes the tangent-space
//! normal maps (`<node>_Normals`), every other port the map connected to
//! its input warped onto the mesh (`<source node>`), `_LOD<n>` before the
//! extension from the second level on. Files go to the build folder in the
//! node's save format, unclamped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use nerthus_engine::{Map, NodeData, Resolution, Terrain};
use nerthus_nodes::mesh_rasterizer::heightmap_to_mesh;
use nerthus_nodes::tangent_bake::{compute_tangent_space_normals, warp_channel, TangentBake};
use nerthus_nodes::transformer::{self, ResizeMethod};
use serde_json::Value;

use super::mesher::lod_count;
use super::{dotnet_extension, flag, input_map, integer, source_node, text, FileResult, PostBuildContext};
use crate::build::OverwriteMode;
use crate::destination::place_in_folder;
use crate::format::{write_map, FileFormat};

/// `SaveDefinition.Format` (Pure Raw when there is none).
fn save_format(node: &NodeData) -> FileFormat {
    let definition = node.base.get("SaveDefinition").filter(|value| !value.is_null());
    let name = definition.and_then(|definition| definition.get("Format")).and_then(Value::as_str);
    name.and_then(FileFormat::from_name).unwrap_or(FileFormat::PureRaw)
}

/// The name shared by the file list and the export: normals belong to
/// the baker, while an additional map keeps its input node's name.
fn port_base_name(terrain: &Terrain, node: &NodeData, port: &str, format: FileFormat) -> Option<String> {
    let extension = format.extension();
    if port == "Out" {
        return flag(node, "GenerateNormals", true).then(|| format!("{}_Normals{extension}", node.name));
    }
    let input_port = port.replace("Out", "In");
    let record = node.port(&input_port)?.record.as_ref()?;
    let source = terrain.nodes.get(&record.from)?;
    Some(format!("{}{extension}", source.name))
}

fn level_path(base: &str, level: usize, folder: &Path, overwrite_mode: OverwriteMode) -> PathBuf {
    let name = if level > 0 {
        let extension = dotnet_extension(base);
        base.replace(extension, &format!("_LOD{level}{extension}"))
    } else {
        base.to_string()
    };
    let increment = overwrite_mode == OverwriteMode::Increment;
    place_in_folder(&name, folder, increment)
}

/// `TextureBaker.UpdateUI` and `MeshWarp.UpdateUI`: the files grouped by
/// exporting port, in the same order and with the same paths as Export.
pub fn planned_files(terrain: &Terrain, node: &NodeData, folder: &Path, overwrite_mode: OverwriteMode) -> Vec<Vec<PathBuf>> {
    let mesher = node.port("In").and_then(|port| port.record.as_ref()).and_then(|record| terrain.nodes.get(&record.from));
    let Some(mesher) = mesher.filter(|source| source.type_name == "Mesher") else {
        return Vec::new();
    };
    let levels = lod_count(text(mesher, "LevelOfDetail", "None")) as usize;
    let format = save_format(node);
    let mut groups = Vec::new();
    for port in node.ports.iter().filter(|port| port.is_output() && port.is_exporting) {
        let Some(base) = port_base_name(terrain, node, &port.name, format) else {
            continue;
        };
        let files = (0..levels).map(|level| level_path(&base, level, folder, overwrite_mode)).collect();
        groups.push(files);
    }
    groups
}

/// The bake for each level of detail: the Mesher's vertex count, halved per
/// level, as a grid mesh over `heightmap` (`TextureBaker.GetNormals` and
/// `GetLODs`).
fn bake_levels(context: &PostBuildContext<'_>, heightmap: &Map, mesher: &NodeData) -> Vec<TangentBake> {
    let levels = lod_count(text(mesher, "LevelOfDetail", "None"));
    let mut vertices = integer(mesher, "VerticesPerSide", 2048) as i32;
    let width = context.terrain.width;
    let height = context.terrain.height;
    let ratio = height / width;
    let mut bakes = Vec::with_capacity(levels.max(0) as usize);
    for _ in 0..levels {
        let size = vertices.max(0) as usize;
        let resized = transformer::resize_with(heightmap, Resolution::square(size), ResizeMethod::HighQuality);
        let mesh = heightmap_to_mesh(resized.data(), size, size, width, height);
        let bake = compute_tangent_space_normals(&mesh, heightmap.channel(0), heightmap.width(), heightmap.height(), ratio);
        bakes.push(bake);
        vertices /= 2;
    }
    bakes
}

/// Every channel of `map` warped by the bake.
fn warped(map: &Map, bake: &TangentBake, warp_width: usize) -> Map {
    let width = map.width();
    let height = map.height();
    let channels = (0..map.channel_count())
        .map(|channel| warp_channel(map.channel(channel), width, height, &bake.warp, warp_width))
        .collect();
    Map::from_channels(map.resolution(), channels)
}

/// Writes one level's map as `TextureBaker.Export` names it: `base`, then
/// `_LOD<level>` before its extension (every occurrence, as .NET's
/// `string.Replace` does), placed in the build folder.
fn write_level(context: &PostBuildContext<'_>, base: &str, level: usize, map: &Map, format: FileFormat, on_file: &mut dyn FnMut(FileResult)) {
    let path = level_path(base, level, context.folder, context.definition.overwrite_mode);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut options = context.preferences.write.clone();
    options.color_space = context.definition.color_space.clone();
    match write_map(format, &path, map, &options) {
        Ok(()) => on_file(FileResult::Written(path)),
        Err(error) => on_file(FileResult::Failed { path, message: error.to_string() }),
    }
}

/// `Node.PostBuild` for one of TextureBaker's or MeshWarp's ports, which
/// `Export` overrides: the normal maps for `Out` (when Generate Normals is
/// on), the warped input for `Out1`…`Out9`.
pub(super) fn export_port(context: &PostBuildContext<'_>, node: &NodeData, port: &str, map: Option<Arc<Map>>, on_file: &mut dyn FnMut(FileResult)) {
    // Build has already failed without a Mesher on `In`.
    let Some(mesher) = source_node(context, node, "In").filter(|source| source.type_name == "Mesher") else {
        return;
    };
    let format = save_format(node);
    let Some(map) = map else {
        let message = format!("{} has no map to export on {port}", node.name);
        on_file(FileResult::Failed { path: context.folder.to_path_buf(), message });
        return;
    };

    if port == "Out" {
        if !flag(node, "GenerateNormals", true) {
            return;
        }
        let base = port_base_name(context.terrain, node, port, format).expect("normal generation is enabled");
        for (level, bake) in bake_levels(context, &map, mesher).into_iter().enumerate() {
            let normal_map = Map::from_channels(map.resolution(), vec![bake.red, bake.green, bake.blue]);
            write_level(context, &base, level, &normal_map, format, on_file);
        }
        return;
    }

    let Some(heightmap) = input_map(context, node, "In") else {
        let message = format!("{} has no Mesher map to bake against", node.name);
        on_file(FileResult::Failed { path: context.folder.to_path_buf(), message });
        return;
    };
    let base = port_base_name(context.terrain, node, port, format).unwrap_or_else(|| format.extension().to_string());
    for (level, bake) in bake_levels(context, &heightmap, mesher).into_iter().enumerate() {
        let level_map = warped(&map, &bake, heightmap.width());
        write_level(context, &base, level, &level_map, format, on_file);
    }
}

#[cfg(test)]
mod tests {
    use nerthus_engine::{Catalogue, ConnectionRecord};
    use serde_json::json;

    use super::*;

    fn bake_terrain(type_name: &str) -> Terrain {
        let catalogue = Catalogue::get();
        let mut mesher = NodeData::new(catalogue.node("Mesher").unwrap(), 1, 1234);
        mesher.values.insert("LevelOfDetail".to_string(), json!("LOD2"));
        let mut source = NodeData::new(catalogue.node("Constant").unwrap(), 2, 1234);
        source.name = "Albedo".to_string();
        let mut baker = NodeData::new(catalogue.node(type_name).unwrap(), 3, 1234);
        baker.name = "Baked".to_string();
        baker.base.insert("SaveDefinition".to_string(), json!({"Format": "PNG16"}));
        baker.add_port("In1", "In");
        baker.add_port("Out1", "Out");
        let mesher_record = ConnectionRecord { from: 1, to: 3, from_port: "Out".to_string(), to_port: "In".to_string(), is_valid: true };
        let albedo_record = ConnectionRecord { from: 2, to: 3, from_port: "Out".to_string(), to_port: "In1".to_string(), is_valid: true };
        baker.port_mut("In").unwrap().record = Some(mesher_record);
        baker.port_mut("In1").unwrap().record = Some(albedo_record);
        let nodes = [mesher, source, baker].into_iter().map(|node| (node.id, node)).collect();
        Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() }
    }

    #[test]
    fn texture_bake_file_lists_match_exported_names_levels_and_selected_ports() {
        let folder = Path::new("Builds");
        for type_name in ["TextureBaker", "MeshWarp"] {
            let mut terrain = bake_terrain(type_name);
            let baker = &terrain.nodes[&3];
            let files = planned_files(&terrain, baker, folder, OverwriteMode::Overwrite);
            let expected = vec![
                vec![folder.join("Baked_Normals.png"), folder.join("Baked_Normals_LOD1.png"), folder.join("Baked_Normals_LOD2.png")],
                vec![folder.join("Albedo.png"), folder.join("Albedo_LOD1.png"), folder.join("Albedo_LOD2.png")],
            ];
            assert_eq!(files, expected);

            let baker = terrain.nodes.get_mut(&3).unwrap();
            baker.values.insert("GenerateNormals".to_string(), json!(false));
            let files = planned_files(&terrain, &terrain.nodes[&3], folder, OverwriteMode::Overwrite);
            assert_eq!(files, expected[1..]);

            let baker = terrain.nodes.get_mut(&3).unwrap();
            baker.port_mut("Out1").unwrap().is_exporting = false;
            assert!(planned_files(&terrain, &terrain.nodes[&3], folder, OverwriteMode::Overwrite).is_empty());
        }
    }

    #[test]
    fn texture_bake_file_list_uses_the_same_incremented_paths_as_writes() {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("nerthus-bake-info-{nanos}"));
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("Baked_Normals.png"), b"").unwrap();
        std::fs::write(folder.join("Albedo_LOD1.png"), b"").unwrap();
        let mut terrain = bake_terrain("TextureBaker");
        let files = planned_files(&terrain, &terrain.nodes[&3], &folder, OverwriteMode::Increment);
        assert_eq!(files[0][0], folder.join("Baked_Normals001.png"));
        assert_eq!(files[1][1], folder.join("Albedo_LOD1001.png"));
        assert_eq!(files[0][1], folder.join("Baked_Normals_LOD1.png"));
        let baker = terrain.nodes.get_mut(&3).unwrap();
        baker.port_mut("In").unwrap().record = None;
        assert!(planned_files(&terrain, &terrain.nodes[&3], &folder, OverwriteMode::Increment).is_empty());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
