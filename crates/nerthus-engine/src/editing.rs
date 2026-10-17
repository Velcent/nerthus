//! Edits the interface makes through node methods: `AddNewPort` (the
//! property area's Add Input command) and SBomber's `UpdateUI`, which adds
//! an input once every input is connected.

use serde_json::Value;

use crate::catalogue::Catalogue;
use crate::project::{ModifierData, NodeData};

/// Whether the node type overrides `AddNewPort` (the property area then
/// shows "This node can add additional input ports").
pub fn can_add_ports(type_name: &str) -> bool {
    matches!(type_name, "Combine" | "Bomber" | "Mixer" | "Layers" | "Construction" | "TextureBaker" | "MeshWarp")
}

fn port_count(node: &NodeData) -> i64 {
    node.values.get("PortCount").and_then(Value::as_i64).unwrap_or(0)
}

fn set_port_count(node: &mut NodeData, count: i64) {
    node.values.insert("PortCount".to_string(), Value::from(count));
}

/// `Node.AddNewPort()`: adds the node's next input (and the ports that go
/// with it); returns the input's name, or `None` at the type's limit.
pub fn add_new_port(node: &mut NodeData) -> Option<String> {
    let mut count = port_count(node);
    match node.type_name.as_str() {
        "Combine" | "Bomber" => {
            while count < 10 {
                count += 1;
                let name = format!("Input{count}");
                if node.port(&name).is_some() {
                    continue;
                }
                node.add_port(&name, "In");
                set_port_count(node, count);
                return Some(name);
            }
            None
        }
        "Mixer" => {
            while count < 30 {
                count += 2;
                let layer = count / 2;
                let name = format!("Layer{layer}");
                let mask_name = format!("Mask{layer}");
                let output_name = format!("MaskOut{layer}");
                let occupied = [&name, &mask_name, &output_name].iter().any(|name| node.port(name).is_some());
                if occupied {
                    continue;
                }
                node.add_port(&name, "In");
                node.add_port(&mask_name, "In");
                node.add_port(&output_name, "Out");
                set_port_count(node, count);
                return Some(name);
            }
            None
        }
        "Layers" => {
            while count < 14 {
                count += 2;
                let layer = count / 2;
                let name = format!("Layer{layer}");
                let mask_name = format!("Mask{layer}");
                if node.port(&name).is_some() || node.port(&mask_name).is_some() {
                    continue;
                }
                node.add_port(&name, "In");
                node.add_port(&mask_name, "In");
                set_port_count(node, count);
                return Some(name);
            }
            None
        }
        "Construction" => {
            while count < 15 {
                count += 1;
                let name = format!("Terrain{count}");
                if node.port(&name).is_some() {
                    continue;
                }
                node.add_port(&name, "In");
                set_port_count(node, count);
                add_terrain_transform(node, count - 1);
                return Some(name);
            }
            None
        }
        "TextureBaker" | "MeshWarp" => {
            while count < 20 {
                let index = count / 2;
                count += 2;
                let name = format!("In{index}");
                let output_name = format!("Out{index}");
                if node.port(&name).is_some() || node.port(&output_name).is_some() {
                    continue;
                }
                node.add_port(&name, "In");
                node.add_port(&output_name, "Out");
                set_port_count(node, count);
                return Some(name);
            }
            None
        }
        _ => None,
    }
}

/// `Transforms.Add(new TerrainTransform { Index = index })`: the type's
/// first default transform is a fresh one.
fn add_terrain_transform(node: &mut NodeData, index: i64) {
    let Some(spec) = Catalogue::get().node(&node.type_name) else {
        return;
    };
    let fresh = spec.defaults.get("Transforms").and_then(Value::as_array).and_then(|transforms| transforms.first()).cloned();
    let Some(mut transform) = fresh else {
        return;
    };
    transform["Index"] = Value::from(index);
    let transforms = node.values.entry("Transforms".to_string()).or_insert_with(|| Value::Array(Vec::new()));
    if let Value::Array(list) = transforms {
        list.push(transform);
    }
}

/// `PropertyArea.AddModifier`: a fresh modifier as the modifier toolbar
/// adds it (`Name` = its type, `Intrinsic`, `Order` = how many the node
/// has), with the members a project saves.
pub fn new_modifier(type_name: &str, order: usize) -> Option<ModifierData> {
    let spec = Catalogue::get().modifier(type_name)?;
    let mut values = spec.defaults.clone();
    values.insert("Name".to_string(), Value::from(type_name));
    if spec.has_ui {
        values.insert("HasUI".to_string(), Value::Bool(true));
    }
    values.insert("Intrinsic".to_string(), Value::Bool(true));
    if order != 0 {
        values.insert("Order".to_string(), Value::from(order as i64));
    }
    let modifier = ModifierData {
        type_name: type_name.to_string(),
        values,
    };
    Some(modifier)
}

/// The modifier's `Name` (what the toolbar's toggles look for).
pub fn modifier_name(modifier: &ModifierData) -> &str {
    modifier.values.get("Name").and_then(Value::as_str).unwrap_or(&modifier.type_name)
}

/// `Modifier.Intrinsic`: added from the modifier toolbar.
pub fn is_intrinsic(modifier: &ModifierData) -> bool {
    modifier.values.get("Intrinsic").and_then(Value::as_bool).unwrap_or(false)
}

/// `Modifier.HasUI`.
pub fn has_ui(modifier: &ModifierData) -> bool {
    modifier.values.get("HasUI").and_then(Value::as_bool).unwrap_or(false)
}

/// `Modifier.Order`.
pub fn modifier_order(modifier: &ModifierData) -> i64 {
    modifier.values.get("Order").and_then(Value::as_i64).unwrap_or(0)
}

/// `SBomber.UpdateUI`: when every `In…` input is connected (as many as
/// `PortCount`) and there are fewer than ten, another input appears.
pub fn grow_inputs(node: &mut NodeData) -> bool {
    if node.type_name != "SBomber" {
        return false;
    }
    let count = port_count(node);
    let connected = node.ports.iter().filter(|port| port.is_input() && port.name.starts_with("In") && port.record.is_some()).count() as i64;
    if connected != count || count >= 10 {
        return false;
    }
    node.add_port(&format!("Input{}", count + 1), "In");
    set_port_count(node, count + 1);
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh(type_name: &str) -> NodeData {
        let spec = Catalogue::get().node(type_name).unwrap();
        NodeData::new(spec, 1, 1)
    }

    #[test]
    fn combine_adds_numbered_inputs_up_to_ten() {
        let mut node = fresh("Combine");
        assert_eq!(add_new_port(&mut node).as_deref(), Some("Input3"));
        for _ in 0..7 {
            assert!(add_new_port(&mut node).is_some());
        }
        assert_eq!(add_new_port(&mut node), None);
        assert_eq!(port_count(&node), 10);
    }

    #[test]
    fn mixer_adds_layer_mask_and_mask_output() {
        let mut node = fresh("Mixer");
        assert_eq!(add_new_port(&mut node).as_deref(), Some("Layer2"));
        let names: Vec<&str> = node.ports.iter().map(|port| port.name.as_str()).collect();
        assert!(names.ends_with(&["Layer2", "Mask2", "MaskOut2"]));
        assert!(node.port("MaskOut2").is_some_and(|port| port.is_output()));
        assert_eq!(port_count(&node), 4);
    }

    #[test]
    fn construction_adds_a_transform() {
        let mut node = fresh("Construction");
        assert_eq!(add_new_port(&mut node).as_deref(), Some("Terrain2"));
        let transforms = node.values["Transforms"].as_array().unwrap();
        assert_eq!(transforms.len(), 2);
        assert_eq!(transforms[1]["Index"], Value::from(1));
    }

    #[test]
    fn added_ports_skip_names_left_by_an_old_reset() {
        let cases = [
            ("Combine", "Input5"),
            ("Bomber", "Input6"),
            ("Mixer", "Layer4"),
            ("Layers", "Layer4"),
            ("Construction", "Terrain4"),
            ("TextureBaker", "In3"),
            ("MeshWarp", "In3"),
        ];
        for (type_name, expected) in cases {
            let mut node = fresh(type_name);
            let original_count = port_count(&node);
            assert!(add_new_port(&mut node).is_some(), "{type_name}");
            assert!(add_new_port(&mut node).is_some(), "{type_name}");
            set_port_count(&mut node, original_count);

            assert_eq!(add_new_port(&mut node).as_deref(), Some(expected), "{type_name}");
            let names: std::collections::HashSet<&str> = node.ports.iter().map(|port| port.name.as_str()).collect();
            assert_eq!(names.len(), node.ports.len(), "{type_name}");
        }
    }

    #[test]
    fn grouped_ports_skip_an_occupied_mask_or_output() {
        let cases = [
            ("Mixer", "MaskOut2", "Layer3"),
            ("Layers", "Mask2", "Layer3"),
            ("TextureBaker", "Out1", "In2"),
            ("MeshWarp", "Out1", "In2"),
        ];
        for (type_name, occupied, expected) in cases {
            let mut node = fresh(type_name);
            node.add_port(occupied, "Out");
            assert_eq!(add_new_port(&mut node).as_deref(), Some(expected), "{type_name}");
            let names: std::collections::HashSet<&str> = node.ports.iter().map(|port| port.name.as_str()).collect();
            assert_eq!(names.len(), node.ports.len(), "{type_name}");
        }
    }

    #[test]
    fn new_modifiers_are_saved_with_their_members() {
        let height = new_modifier("Height", 0).unwrap();
        let keys: Vec<&str> = height.values.keys().map(String::as_str).collect();
        assert_eq!(keys, ["Range", "Falloff", "Distort", "Name", "HasUI", "Intrinsic"]);
        let invert = new_modifier("Invert", 2).unwrap();
        assert!(!has_ui(&invert));
        assert_eq!(modifier_order(&invert), 2);
        assert!(is_intrinsic(&invert));
    }

    #[test]
    fn sbomber_grows_when_every_input_is_connected() {
        let mut node = fresh("SBomber");
        assert!(!grow_inputs(&mut node));
        for name in ["In", "Input2"] {
            let port = node.port_mut(name).unwrap();
            port.record = Some(crate::project::ConnectionRecord {
                from: 7,
                to: 1,
                from_port: "Out".to_string(),
                to_port: name.to_string(),
                is_valid: true,
            });
        }
        assert!(grow_inputs(&mut node));
        assert!(node.port("Input3").is_some());
        assert!(!grow_inputs(&mut node));
    }
}
