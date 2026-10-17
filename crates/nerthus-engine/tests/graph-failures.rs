use std::sync::Arc;

use nerthus_engine::{BuildContext, BuildResult, Catalogue, ConnectionRecord, Engine, Map, NodeBuilder, NodeData, NodeError, NodeState, Registry, Resolution, Terrain};
use serde_json::Value;

struct Source;

impl NodeBuilder for Source {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        context.commit(Map::filled(context.resolution, 0.5))?;
        if context.node.values.get("Fail") == Some(&Value::Bool(true)) {
            context.update_member("Partial", Value::Bool(true));
            let committed = context.committed("Out").unwrap();
            let _ = committed.data()[context.resolution.area()];
        }
        Ok(())
    }
}

struct PassThrough;

impl NodeBuilder for PassThrough {
    fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
        let input = context.require_input("In")?;
        context.commit((*input).clone())
    }
}

fn fresh_node(type_name: &str, id: i32) -> NodeData {
    let spec = Catalogue::get().node(type_name).unwrap();
    NodeData::new(spec, id, 1)
}

#[test]
fn a_panicking_node_discards_outputs_and_other_nodes_keep_building() {
    let mut source = fresh_node("Constant", 1);
    source.values.insert("Fail".to_string(), Value::Bool(true));
    let mut downstream = fresh_node("Invert", 2);
    let connection = ConnectionRecord {
        from: 1,
        to: 2,
        from_port: "Out".to_string(),
        to_port: "In".to_string(),
        is_valid: true,
    };
    downstream.port_mut("In").unwrap().record = Some(connection);
    let unrelated = fresh_node("Constant", 3);
    let nodes = [(1, source), (2, downstream), (3, unrelated)].into();
    let mut terrain = Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() };
    let mut registry = Registry::default();
    registry.add_node("Constant", Source);
    registry.add_node("Invert", PassThrough);
    let mut engine = Engine::new(Arc::new(registry));
    let resolution = Resolution::square(4);
    let stale = Arc::new(Map::filled(resolution, 0.25));
    engine.cache.store(1, "Out", stale);

    let result = engine.build(&terrain, 2, resolution);
    let validation = NodeError::Validation("The port In returned bad or no data.".to_string());
    assert_eq!(result, BuildResult::Failed(validation));
    assert_eq!(engine.state(1), NodeState::Error);
    assert_eq!(engine.state(2), NodeState::Error);
    let source_error = NodeError::Failed("Index was outside the bounds of the array.".to_string());
    assert_eq!(engine.error(1), Some(&source_error));
    assert!(engine.cache.get(1, "Out").is_none());
    assert!(engine.cache.get(2, "Out").is_none());
    assert!(engine.take_member_updates().is_empty());

    assert_eq!(engine.build(&terrain, 3, resolution), BuildResult::Succeeded);
    assert_eq!(engine.state(3), NodeState::Built);
    terrain.nodes[&1].values.insert("Fail".to_string(), Value::Bool(false));
    assert_eq!(engine.build(&terrain, 2, resolution), BuildResult::Succeeded);
    assert_eq!(engine.state(1), NodeState::Built);
    assert_eq!(engine.state(2), NodeState::Built);
    assert!(engine.error(1).is_none());
    assert!(engine.error(2).is_none());
}

struct InvalidSource;

impl NodeBuilder for InvalidSource {
    fn check_validity(&self, _context: &BuildContext) -> Result<(), NodeError> {
        panic!("validation failed");
    }

    fn build(&self, _context: &mut BuildContext) -> Result<(), NodeError> {
        unreachable!("validation should fail before building");
    }
}

#[test]
fn panics_during_validation_also_become_node_failures() {
    let node = fresh_node("Constant", 1);
    let nodes = [(1, node)].into();
    let terrain = Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() };
    let mut registry = Registry::default();
    registry.add_node("Constant", InvalidSource);
    let mut engine = Engine::new(Arc::new(registry));
    let failure = NodeError::Failed("validation failed".to_string());

    assert_eq!(engine.build(&terrain, 1, Resolution::square(4)), BuildResult::Failed(failure));
    assert_eq!(engine.state(1), NodeState::Error);
}
