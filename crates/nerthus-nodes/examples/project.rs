//! Builds one node of a `.terrain` project the way the oracle's `project`
//! command builds it (the file opened as File › Open opens it, the node
//! built with everything upstream), so the outputs can be compared file by
//! file.
//!
//! usage: project <file.terrain> <node id> <resolution> <output directory> [--community] [--macros <directory>] [--all]
//!
//! `--community` builds as a Community build does (as the oracle runs):
//! the build loop skips bindings and expressions, so
//! they are dropped here; variables still change.
//! `--macros` reads the project's macros from that directory.
//! `--all` also writes every node of the build order to
//! `<output directory>/<id>/`.
//! `--isolate <directory>` builds every node of the build order on its own
//! from another run's `--all` outputs (`<directory>/<id>/`): the nodes before
//! it are loaded from there, so each node's result shows its own
//! differences only. With `--only`, just the node itself.

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;
use std::sync::Arc;

use nerthus_engine::macros::load_project_macros;
use nerthus_engine::{BuildResult, Engine, Map, Project, RenderIntent, Resolution};
use serde_json::json;

fn write_raw(path: &Path, values: &[f32]) {
    let mut bytes = Vec::with_capacity(values.len() * 4);
    for value in values {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    fs::write(path, bytes).expect("output file should be writable");
}

fn main() -> ExitCode {
    let mut arguments: Vec<String> = env::args().skip(1).collect();
    let community = arguments.iter().any(|argument| argument == "--community");
    arguments.retain(|argument| argument != "--community");
    let write_all = arguments.iter().any(|argument| argument == "--all");
    arguments.retain(|argument| argument != "--all");
    let only = arguments.iter().any(|argument| argument == "--only");
    arguments.retain(|argument| argument != "--only");
    let mut macro_directory = None;
    if let Some(position) = arguments.iter().position(|argument| argument == "--macros") {
        macro_directory = arguments.get(position + 1).cloned();
        arguments.drain(position..(position + 2).min(arguments.len()));
    }
    let mut isolate_directory = None;
    if let Some(position) = arguments.iter().position(|argument| argument == "--isolate") {
        isolate_directory = arguments.get(position + 1).cloned();
        arguments.drain(position..(position + 2).min(arguments.len()));
    }
    if arguments.len() < 4 {
        eprintln!("usage: project <file.terrain> <node id> <resolution> <output directory>");
        return ExitCode::FAILURE;
    }
    let file = Path::new(&arguments[0]);
    let node_id: i32 = arguments[1].parse().expect("the node id should be a number");
    let size: usize = arguments[2].parse().expect("the resolution should be a number");
    let output_directory = Path::new(&arguments[3]);
    fs::create_dir_all(output_directory).expect("output directory should be creatable");

    let text = fs::read_to_string(file).expect("the project should be readable");
    let project = match Project::parse(&text) {
        Ok(project) => project,
        Err(error) => {
            eprintln!("build failed: {error}");
            return ExitCode::from(2);
        }
    };
    let asset = &project.assets[0];
    let terrain = &asset.terrain;
    if !terrain.nodes.contains_key(&node_id) {
        eprintln!("build failed: the project has no node {node_id}");
        return ExitCode::from(2);
    }

    let registry = Arc::new(nerthus_nodes::registry());
    let mut engine = Engine::new(registry);
    engine.automation = asset.automation.clone();
    if community {
        engine.automation.bindings.clear();
        engine.automation.expressions.clear();
    }
    if let Some(directory) = macro_directory {
        let (library, problems) = load_project_macros(&project.macro_ids(), Path::new(&directory));
        for (id, problem) in problems {
            println!("macro {id} error: {problem}");
        }
        engine.macros = library;
    }

    let resolution = Resolution::square(size);
    let file_name = file.file_name().and_then(|name| name.to_str()).unwrap_or_default();
    if let Some(directory) = isolate_directory {
        let order = Engine::build_order(terrain, node_id);
        for (position, &id) in order.iter().enumerate() {
            if only && id != node_id {
                continue;
            }
            let mut isolated = Engine::new(engine.registry());
            isolated.automation = engine.automation.clone();
            isolated.macros = engine.macros.clone();
            for &earlier in &order[..position] {
                let earlier_directory = Path::new(&directory).join(earlier.to_string());
                if let Some(outputs) = read_outputs(&earlier_directory, resolution) {
                    isolated.load_baked(earlier, outputs);
                    isolated.mark_built(earlier);
                }
            }
            let started = std::time::Instant::now();
            let result = isolated.build(terrain, id, resolution);
            println!("built node {id} alone at {size} in {} ms: {result:?}", started.elapsed().as_millis());
            if let Some(error) = isolated.error(id) {
                println!("node {id} error: {error}");
            }
            let node_directory = output_directory.join(id.to_string());
            fs::create_dir_all(&node_directory).expect("output directory should be creatable");
            write_outputs(&isolated, id, file_name, size, &node_directory);
        }
        return ExitCode::SUCCESS;
    }
    let started = std::time::Instant::now();
    let result = engine.build(terrain, node_id, resolution);
    println!("built node {node_id} at {size} in {} ms: {result:?}", started.elapsed().as_millis());
    for id in Engine::build_order(terrain, node_id) {
        if let Some(error) = engine.error(id) {
            println!("node {id} error: {error}");
        }
    }
    if let BuildResult::Failed(error) = result {
        eprintln!("build failed: {error}");
    }

    write_outputs(&engine, node_id, file_name, size, output_directory);
    if write_all {
        for id in Engine::build_order(terrain, node_id) {
            let node_directory = output_directory.join(id.to_string());
            fs::create_dir_all(&node_directory).expect("output directory should be creatable");
            write_outputs(&engine, id, file_name, size, &node_directory);
        }
    }
    ExitCode::SUCCESS
}

/// A node's ports as [`write_outputs`] wrote them (`None` when the
/// directory has no manifest).
fn read_outputs(directory: &Path, resolution: Resolution) -> Option<Vec<(String, Map)>> {
    let manifest_text = fs::read_to_string(directory.join("manifest.json")).ok()?;
    let manifest: serde_json::Value = serde_json::from_str(&manifest_text).ok()?;
    let mut outputs = Vec::new();
    for output in manifest["outputs"].as_array()? {
        let port = output["port"].as_str()?;
        let channel_count = output["channels"].as_u64()? as usize;
        let mut channels = Vec::new();
        for channel in 0..channel_count {
            let file = if channel_count == 1 { format!("{port}.raw") } else { format!("{port}.c{channel}.raw") };
            let bytes = fs::read(directory.join(file)).ok()?;
            let values: Vec<f32> = bytes.as_chunks::<4>().0.iter().map(|chunk| f32::from_le_bytes(*chunk)).collect();
            channels.push(values);
        }
        let mut map = Map::from_channels(resolution, channels);
        if let Some(intent) = output["intent"].as_str().and_then(RenderIntent::from_name) {
            map.info.render_intent = intent;
        }
        outputs.push((port.to_string(), map));
    }
    Some(outputs)
}

/// Writes a node's cached ports as raw planes plus `manifest.json`.
fn write_outputs(engine: &Engine, node_id: i32, file_name: &str, size: usize, output_directory: &Path) {
    let mut outputs = Vec::new();
    for (port, map) in engine.cache.ports(node_id) {
        let channel_count = map.channel_count();
        for channel in 0..channel_count {
            let file = if channel_count == 1 { format!("{port}.raw") } else { format!("{port}.c{channel}.raw") };
            write_raw(&output_directory.join(file), map.channel(channel));
        }
        outputs.push(json!({"port": port, "channels": channel_count, "intent": map.info.render_intent.name()}));
    }
    let label = format!("{file_name}#{node_id}");
    let manifest = json!({"node": label, "resolution": size, "outputs": outputs});
    let manifest_text = serde_json::to_string_pretty(&manifest).expect("manifest serializes");
    fs::write(output_directory.join("manifest.json"), manifest_text).expect("manifest should be writable");
    println!("{}", serde_json::to_string(&manifest["outputs"]).expect("outputs serialize"));
}
