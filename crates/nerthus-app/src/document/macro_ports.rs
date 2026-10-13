//! Vars › Create Connected Nodes › For Macros (`GraphArea.CreateMacroPorts`):
//! a MacroPort feeding each unconnected input of the chosen nodes, 300 to
//! the node's left, and one fed by each unconnected output, 500 to its
//! right, the ports of a side 100 apart.

use nerthus_engine::{Catalogue, NodeData, RenderIntent};
use serde_json::json;

use super::Document;

/// `CreateMacroPorts`' offsets from the node.
const INPUT_OFFSET: f32 = -300.0;
const OUTPUT_OFFSET: f32 = 500.0;
const PORT_SPACING: f32 = 100.0;

/// `MacroPort.ToDisplayIntent`: Default and Raw show as heightfields.
fn display_intent(intent: RenderIntent) -> &'static str {
    match intent {
        RenderIntent::Mask => "Mask",
        RenderIntent::Color => "Color",
        RenderIntent::Default | RenderIntent::Heightfield | RenderIntent::Raw => "Heightfield",
    }
}

/// A MacroPort to make: where, its name and settings, and what it joins.
struct PlannedPort {
    position: [f32; 2],
    name: String,
    output: bool,
    display: &'static str,
    optional: bool,
    node: i32,
    port: String,
    tab: usize,
}

impl Document {
    /// `CreateMacroPorts(nodes, in, out, primaryOnly: false)`: the nodes
    /// from left to right, each port named after the port it joins, or
    /// `Node_Port` when several nodes are chosen or a MacroPort already has
    /// that name. `output_intent` tells the intent a built output was left
    /// with (`Port.RenderIntent`). One undo step. Returns the new nodes.
    pub fn create_macro_ports(&mut self, nodes: &[i32], inputs: bool, outputs: bool, output_intent: impl Fn(i32, &str) -> Option<RenderIntent>) -> Vec<i32> {
        let mut chosen: Vec<&NodeData> = nodes.iter().filter_map(|id| self.node(*id)).collect();
        chosen.sort_by(|left, right| left.position[0].total_cmp(&right.position[0]));
        let several = nodes.len() > 1;
        let mut taken: Vec<String> = self.terrain().nodes.values().filter(|node| node.type_name == "MacroPort").map(|node| node.name.clone()).collect();
        let mut planned = Vec::new();
        for node in chosen {
            let outgoing = self.outgoing(node.id);
            let sides = [(false, inputs, INPUT_OFFSET), (true, outputs, OUTPUT_OFFSET)];
            for (output, wanted, offset) in sides {
                if !wanted {
                    continue;
                }
                let mut below = 0.0;
                for port in node.ports.iter().filter(|port| if output { port.is_output() } else { port.is_input() }) {
                    let connected = if output {
                        outgoing.iter().any(|(_, _, from_port)| *from_port == port.name)
                    } else {
                        port.record.as_ref().is_some_and(|record| self.node(record.from).is_some())
                    };
                    if connected {
                        continue;
                    }
                    let built = if output { output_intent(node.id, &port.name) } else { None };
                    let intent = built.or(port.declared_intent).unwrap_or(RenderIntent::Default);
                    let clash = taken.contains(&port.name);
                    let name = if clash || several { format!("{}_{}", node.name, port.name) } else { port.name.clone() };
                    taken.push(name.clone());
                    planned.push(PlannedPort {
                        position: [node.position[0] + offset, node.position[1] + below],
                        name,
                        output,
                        display: display_intent(intent),
                        optional: !port.is_required(),
                        node: node.id,
                        port: port.name.clone(),
                        tab: super::node_tab(node),
                    });
                    below += PORT_SPACING;
                }
            }
        }
        let Some(spec) = Catalogue::get().node("MacroPort") else {
            return Vec::new();
        };
        if planned.is_empty() {
            return Vec::new();
        }
        self.checkpoint("Macro ports created");
        let mut made = Vec::with_capacity(planned.len());
        for plan in planned {
            let id = self.new_node_id();
            let seed = self.random_seed();
            let mut macro_port = NodeData::new(spec, id, seed);
            macro_port.position = plan.position;
            macro_port.name = plan.name;
            let direction = if plan.output { "Output" } else { "Input" };
            macro_port.values.insert("Direction".to_string(), json!(direction));
            macro_port.values.insert("Display".to_string(), json!(plan.display));
            macro_port.values.insert("IsOptional".to_string(), json!(plan.optional));
            // On the graph tab of the node it joins (the one shown).
            if plan.tab != 0 {
                macro_port.set_member("GraphIndex", json!(plan.tab));
            }
            self.project.terrain_mut().nodes.insert(id, macro_port);
            self.mark_stale(id);
            if plan.output {
                self.link(plan.node, &plan.port, id, "In");
            } else {
                self.link(id, "Out", plan.node, &plan.port);
            }
            made.push(id);
        }
        made
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_follows_the_port_intent() {
        assert_eq!(display_intent(RenderIntent::Mask), "Mask");
        assert_eq!(display_intent(RenderIntent::Raw), "Heightfield");
    }

    #[test]
    fn unconnected_ports_get_macro_ports_beside_their_node() {
        let mut document = Document::new();
        let mountain = document.add_node("Mountain", [0.0, 0.0]).unwrap();
        let erosion = document.add_node("ErosionPro", [300.0, 50.0]).unwrap();
        document.connect(mountain, "Out", erosion, "In");
        let built = |_: i32, port: &str| (port == "Wear").then_some(RenderIntent::Mask);
        let made = document.create_macro_ports(&[erosion], true, true, built);
        let ports: Vec<(String, [f32; 2], String, String, bool)> = made
            .iter()
            .map(|id| {
                let node = document.node(*id).unwrap();
                let value = |name: &str| node.values.get(name).and_then(|value| value.as_str()).unwrap_or_default().to_string();
                let optional = node.values.get("IsOptional").and_then(|value| value.as_bool()).unwrap_or(false);
                (node.name.clone(), node.position, value("Direction"), value("Display"), optional)
            })
            .collect();
        // In is fed already; Precipitation is optional. Out and the other
        // outputs feed nothing; Flow and Deposits are masks as made, Wear
        // as built.
        let expected = vec![
            ("Precipitation".to_string(), [0.0, 50.0], "Input".to_string(), "Heightfield".to_string(), true),
            ("Out".to_string(), [800.0, 50.0], "Output".to_string(), "Heightfield".to_string(), true),
            ("Flow".to_string(), [800.0, 150.0], "Output".to_string(), "Mask".to_string(), true),
            ("Wear".to_string(), [800.0, 250.0], "Output".to_string(), "Mask".to_string(), true),
            ("Deposits".to_string(), [800.0, 350.0], "Output".to_string(), "Mask".to_string(), true),
        ];
        assert_eq!(ports, expected);
        let fed = document.node(erosion).unwrap().port("Precipitation").unwrap().record.clone().unwrap();
        assert_eq!((fed.from, fed.from_port.as_str()), (made[0], "Out"));
        assert_eq!(document.undo_labels()[0], "Macro ports created");
    }
}
