use std::sync::Arc;

use indexmap::IndexMap;
use nerthus_engine::{
    BuildContext, BuildResult, Catalogue, ConnectionRecord, Engine, Map, NodeBuilder, NodeData,
    NodeError, Resolution, Terrain,
};
use serde_json::{Map as JsonObject, Value, json};

const TERRAIN_INPUT: i32 = 1;
const MASK_INPUT: i32 = 2;
const LATER_INPUT: i32 = 3;
const LEGACY_NODE: i32 = 100;
const POSITIONAL_NODE: i32 = 101;
const NAMED_NODE: i32 = 102;

struct Fixture;

impl NodeBuilder for Fixture {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let map = match context.node_id() {
            TERRAIN_INPUT => {
                let mut heights = Vec::with_capacity(context.resolution.area());
                let size = context.resolution.x;
                for y in 0..size {
                    for x in 0..size {
                        let slope = (x + y) as f32 / (size * 3) as f32;
                        let detail = ((x * 7 + y * 13) % 11) as f32 / 50.0;
                        heights.push(slope + detail);
                    }
                }
                Map::from_data(context.resolution, heights)
            }
            MASK_INPUT => Map::filled(context.resolution, 1.0),
            LATER_INPUT => Map::filled(context.resolution, 0.0),
            _ => unreachable!(),
        };
        context.commit(map)
    }
}

fn connect(node: &mut NodeData, port: &str, source: i32) {
    let record = ConnectionRecord {
        from: source,
        to: node.id,
        from_port: "Out".to_string(),
        to_port: port.to_string(),
        is_valid: true,
    };
    node.port_mut(port).unwrap().record = Some(record);
}

fn node_with_mask(type_name: &str, id: i32) -> NodeData {
    let spec = Catalogue::get().node(type_name).unwrap();
    let mut node = NodeData::new(spec, id, 1234);
    node.add_port("Mask", "In");
    connect(&mut node, "In", TERRAIN_INPUT);
    connect(&mut node, "Mask", MASK_INPUT);
    match type_name {
        "ErosionPro" => {
            node.values.insert("Duration".to_string(), json!(0.1));
            node.values
                .insert("ErosionScale".to_string(), json!(2000.0));
        }
        "Cellular3D" => {
            node.values.insert("Gap".to_string(), json!(0.2));
        }
        "ThermalShaper" => {
            node.values.insert("Scale".to_string(), json!(200.0));
        }
        "Debris" => {
            node.values.insert("DebrisAmount".to_string(), json!(20));
            node.values
                .insert("RenderStillRocks".to_string(), Value::Bool(true));
            connect(&mut node, "Emitter", MASK_INPUT);
        }
        _ => {}
    }
    node
}

fn build_variants(type_name: &str, later_port: &str, mask_connected: bool) -> Engine {
    let mut legacy = node_with_mask(type_name, LEGACY_NODE);
    legacy.ports.retain(|port| port.name != later_port);
    let old_version = if type_name == "Debris" { 2 } else { 1 };
    legacy
        .base
        .insert("Version".to_string(), Value::from(old_version));
    let spec = Catalogue::get().node(type_name).unwrap();
    nerthus_engine::migrate::migrate_node(&mut legacy, spec);
    connect(&mut legacy, later_port, LATER_INPUT);

    let mut positional = node_with_mask(type_name, POSITIONAL_NODE);
    let mut named = node_with_mask(type_name, NAMED_NODE);
    connect(&mut named, later_port, LATER_INPUT);
    if mask_connected {
        connect(&mut positional, later_port, MASK_INPUT);
    } else {
        for node in [&mut legacy, &mut positional, &mut named] {
            node.port_mut("Mask").unwrap().record = None;
        }
    }

    let mut nodes = IndexMap::new();
    let fixture_spec = Catalogue::get().node("Constant").unwrap();
    for id in [TERRAIN_INPUT, MASK_INPUT, LATER_INPUT] {
        nodes.insert(id, NodeData::new(fixture_spec, id, 1));
    }
    for node in [legacy, positional, named] {
        nodes.insert(node.id, node);
    }
    let terrain = Terrain {
        id: "legacy-inputs".to_string(),
        nodes,
        width: 5000.0,
        height: 2600.0,
        extra: JsonObject::new(),
    };
    let mut registry = nerthus_nodes::registry();
    registry.add_node("Constant", Fixture);
    let mut engine = Engine::new(Arc::new(registry));
    let resolution = Resolution::square(32);
    for id in [LEGACY_NODE, POSITIONAL_NODE, NAMED_NODE] {
        let result = engine.build(&terrain, id, resolution);
        assert!(
            matches!(result, BuildResult::Succeeded),
            "{type_name}: {result:?}"
        );
    }
    engine
}

fn output_bits(engine: &Engine, node: i32) -> Vec<(String, Vec<u32>)> {
    let mut outputs = Vec::new();
    for (port, map) in engine.cache.ports(node) {
        let bits = map.data().iter().map(|value| value.to_bits()).collect();
        outputs.push((port.clone(), bits));
    }
    outputs.sort_by(|first, second| first.0.cmp(&second.0));
    outputs
}

#[test]
fn migrated_nodes_read_masks_before_later_inputs() {
    nerthus_compute::set_cpu_only(true);
    let cases = [
        ("ErosionPro", "Precipitation"),
        ("TextureBase", "Guide"),
        ("Cellular3D", "Gap"),
        ("ThermalShaper", "Intensity"),
        ("Debris", "Removal"),
    ];
    for (type_name, later_port) in cases {
        let engine = build_variants(type_name, later_port, true);
        let actual = output_bits(&engine, LEGACY_NODE);
        let expected = output_bits(&engine, POSITIONAL_NODE);
        let named = output_bits(&engine, NAMED_NODE);
        assert!(
            actual == expected,
            "{type_name} must read the mask's input slot"
        );
        assert!(
            actual != named,
            "{type_name} fixture must distinguish input names from positions"
        );
    }
}

#[test]
fn disconnected_masks_keep_their_input_slots() {
    nerthus_compute::set_cpu_only(true);
    let cases = [
        ("ErosionPro", "Precipitation"),
        ("TextureBase", "Guide"),
        ("Cellular3D", "Gap"),
        ("ThermalShaper", "Intensity"),
        ("Debris", "Removal"),
    ];
    for (type_name, later_port) in cases {
        let engine = build_variants(type_name, later_port, false);
        let actual = output_bits(&engine, LEGACY_NODE);
        let expected = output_bits(&engine, POSITIONAL_NODE);
        assert!(
            actual == expected,
            "{type_name} must leave the disconnected mask's input slot empty"
        );
    }
}
