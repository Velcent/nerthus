use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use nerthus_engine::{BuildContext, BuildResult, Catalogue, ConnectionRecord, Engine, Map, NodeBuilder, NodeData, NodeError, NodeState, Registry, Resolution, Terrain};
use serde_json::Value;

struct Source {
    process_builds: Arc<AtomicUsize>,
}

impl NodeBuilder for Source {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        if context.node_id() == 4 {
            let previous_builds = self.process_builds.fetch_add(1, Ordering::SeqCst);
            if previous_builds == 0 {
                panic!("loop process failed");
            }
        }
        context.commit(Map::filled(context.resolution, 0.5))
    }
}

struct PassThrough;

impl NodeBuilder for PassThrough {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        if context.loop_in_progress() {
            return Ok(());
        }
        let input = context.require_input("In")?;
        context.commit((*input).clone())
    }
}

fn fresh_node(type_name: &str, id: i32) -> NodeData {
    let spec = Catalogue::get().node(type_name).unwrap();
    NodeData::new(spec, id, 1)
}

fn connect(node: &mut NodeData, input: &str, source: i32) {
    let record = ConnectionRecord {
        from: source,
        to: node.id,
        from_port: "Out".to_string(),
        to_port: input.to_string(),
        is_valid: true,
    };
    node.port_mut(input).unwrap().record = Some(record);
}

#[test]
fn loop_process_panics_stop_the_loop_and_allow_another_build() {
    let source = fresh_node("Constant", 1);
    let mut loop_begin = fresh_node("LoopBegin", 2);
    connect(&mut loop_begin, "In", 1);
    let mut loop_end = fresh_node("LoopEnd", 3);
    loop_end.values.insert("Primed".to_string(), Value::Bool(true));
    loop_end.values.insert("Loops".to_string(), Value::from(3));
    connect(&mut loop_end, "In", 2);
    connect(&mut loop_end, "Processes", 4);
    let process = fresh_node("Constant", 4);
    let nodes = [(1, source), (2, loop_begin), (3, loop_end), (4, process)].into();
    let terrain = Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() };
    let process_builds = Arc::new(AtomicUsize::new(0));
    let source_builder = Source { process_builds: process_builds.clone() };
    let mut registry = Registry::default();
    registry.add_node("Constant", source_builder);
    registry.add_node("LoopBegin", PassThrough);
    registry.add_node("LoopEnd", PassThrough);
    let mut engine = Engine::new(Arc::new(registry));
    let resolution = Resolution::square(4);
    let cached_process = Arc::new(Map::filled(resolution, 0.5));
    engine.cache.store(4, "Out", cached_process);
    engine.mark_built(4);

    assert_eq!(engine.build(&terrain, 3, resolution), BuildResult::Succeeded);
    assert_eq!(process_builds.load(Ordering::SeqCst), 1);
    assert_eq!(engine.state(3), NodeState::Built);
    assert!(engine.error(3).is_none());
    assert!(engine.cache.get(3, "Out").is_some());

    engine.invalidate(&terrain, 3);
    assert_eq!(engine.build(&terrain, 3, resolution), BuildResult::Succeeded);
    assert_eq!(process_builds.load(Ordering::SeqCst), 3);
    assert!(engine.cache.get(3, "Out").is_some());
}
