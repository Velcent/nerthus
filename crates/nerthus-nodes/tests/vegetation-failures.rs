use std::sync::Arc;

use nerthus_engine::{BuildResult, Catalogue, ConnectionRecord, Engine, NodeData, NodeError, NodeState, Resolution, Terrain};
use serde_json::Value;

// This seed's second float rounds to 1.0, putting the first sample at
// y == width in both RandomNoises.FixedWhite and FixedWhiteMapped.
const BOTTOM_EDGE_SEED: i32 = 23_676_416;

#[test]
fn vegetation_outside_the_map_fails_only_its_branch_and_can_be_rebuilt() {
    let catalogue = Catalogue::get();
    let resolution = Resolution::square(32);
    for type_name in ["Trees", "Shrubs"] {
        let input = NodeData::new(catalogue.node("Constant").unwrap(), 1, 1);
        let mut vegetation = NodeData::new(catalogue.node(type_name).unwrap(), 2, BOTTOM_EDGE_SEED);
        vegetation.values.insert("IsRelative".to_string(), Value::Bool(false));
        let vegetation_input = ConnectionRecord {
            from: 1,
            to: 2,
            from_port: "Out".to_string(),
            to_port: "In".to_string(),
            is_valid: true,
        };
        vegetation.port_mut("In").unwrap().record = Some(vegetation_input);
        let mut downstream = NodeData::new(catalogue.node("Invert").unwrap(), 3, 1);
        let downstream_input = ConnectionRecord {
            from: 2,
            to: 3,
            from_port: "Out".to_string(),
            to_port: "In".to_string(),
            is_valid: true,
        };
        downstream.port_mut("In").unwrap().record = Some(downstream_input);
        let unrelated = NodeData::new(catalogue.node("Constant").unwrap(), 4, 1);
        let nodes = [(1, input), (2, vegetation), (3, downstream), (4, unrelated)].into();
        let mut terrain = Terrain { id: String::new(), nodes, width: 5000.0, height: 2500.0, extra: Default::default() };
        let mut engine = Engine::new(Arc::new(nerthus_nodes::registry()));

        let result = engine.build(&terrain, 3, resolution);
        let bad_input = NodeError::Validation("The port In returned bad or no data.".to_string());
        assert_eq!(result, BuildResult::Failed(bad_input), "{type_name}");
        let outside_array = NodeError::Failed("Index was outside the bounds of the array.".to_string());
        assert_eq!(engine.error(2), Some(&outside_array), "{type_name}");
        assert_eq!(engine.state(1), NodeState::Built);
        assert_eq!(engine.state(2), NodeState::Error);
        assert_eq!(engine.state(3), NodeState::Error);
        assert!(engine.cache.ports(2).next().is_none());
        assert_eq!(engine.build(&terrain, 4, resolution), BuildResult::Succeeded);

        // Correcting the node can rebuild the failed branch on the same
        // engine, as the editor and the persistent build host need to do.
        let vegetation = &mut terrain.nodes[&2];
        vegetation.values.insert("TreeCount".to_string(), Value::from(0));
        vegetation.values.insert("ShrubCount".to_string(), Value::from(0));
        vegetation.values.insert("Random".to_string(), Value::from(0.0));
        assert_eq!(engine.build(&terrain, 3, resolution), BuildResult::Succeeded, "{type_name}");
        assert!(engine.error(2).is_none());
        assert!(engine.error(3).is_none());
    }
}
