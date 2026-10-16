//! Progress reported by a node while its builder runs.

/// A node's share of its work done, or its current operation's message.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildProgress {
    Fraction(f32),
    Text(String),
}

pub type BuildProgressFn = dyn Fn(i32, BuildProgress) + Send + Sync;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use crate::{BuildContext, BuildResult, Catalogue, Engine, Map, NodeBuilder, NodeData, NodeError, Registry, Resolution, Terrain};

    struct ReportingNode;

    impl NodeBuilder for ReportingNode {
        fn build(&self, context: &mut BuildContext) -> Result<(), NodeError> {
            context.report_progress(0.25);
            context.report_progress_text("Rendering pass 2 of 5");
            context.report_progress(f32::NAN);
            context.report_progress(2.0);
            context.commit(Map::new(context.resolution, 1))
        }
    }

    #[test]
    fn node_progress_reaches_the_engine_listener_and_cached_nodes_stay_quiet() {
        let mut registry = Registry::default();
        registry.add_node("Mountain", ReportingNode);
        let mut engine = Engine::new(Arc::new(registry));
        let events = Arc::new(Mutex::new(Vec::new()));
        let received = events.clone();
        let report = move |node, progress| received.lock().unwrap().push((node, progress));
        engine.progress = Some(Arc::new(report));

        let spec = Catalogue::get().node("Mountain").unwrap();
        let node = NodeData::new(spec, 42, 0);
        let terrain = Terrain {
            id: String::new(),
            nodes: indexmap::IndexMap::from([(42, node)]),
            width: 5000.0,
            height: 2600.0,
            extra: serde_json::Map::new(),
        };
        let resolution = Resolution::square(4);
        assert_eq!(engine.build(&terrain, 42, resolution), BuildResult::Succeeded);
        let expected = vec![
            (42, BuildProgress::Fraction(0.0)),
            (42, BuildProgress::Fraction(0.25)),
            (42, BuildProgress::Text("Rendering pass 2 of 5".to_string())),
            (42, BuildProgress::Fraction(1.0)),
            (42, BuildProgress::Fraction(1.0)),
        ];
        assert_eq!(*events.lock().unwrap(), expected);
        events.lock().unwrap().clear();
        assert_eq!(engine.build(&terrain, 42, resolution), BuildResult::AlreadyBuilt);
        assert!(events.lock().unwrap().is_empty());
    }
}
