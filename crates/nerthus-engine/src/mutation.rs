//! Mutations: a node's seeds moved on by the mutation number, so that the
//! terrain builds as a variant of itself. `nerthus-build --seed` sets the
//! number and mutates every node once before the build; a `LoopEnd` with
//! Mutate Seeds mutates the nodes of its loop again before every pass after
//! the first. The build folder's `[Mutation]` token shows the number.

use serde_json::Value;

use crate::catalogue::{Catalogue, ValueType};
use crate::project::{NodeData, Terrain};

/// A mutation's changes to a node: every integer `Seed` parameter with
/// `mutation` added (wrapping, as C#'s unchecked `int` addition does).
pub fn mutated_seeds(node: &NodeData, mutation: i32) -> Vec<(String, Value)> {
    let Some(spec) = Catalogue::get().node(&node.type_name) else {
        return Vec::new();
    };
    let mut changes = Vec::new();
    for parameter in spec.parameters.iter().filter(|parameter| parameter.is_seed() && parameter.value_type == ValueType::Int) {
        let current = node.values.get(&parameter.name).and_then(Value::as_i64).unwrap_or(0) as i32;
        let mutated = current.wrapping_add(mutation);
        changes.push((parameter.name.clone(), Value::from(mutated)));
    }
    changes
}

/// Every node of the terrain mutated once.
pub fn mutate_terrain(terrain: &mut Terrain, mutation: i32) {
    for node in terrain.nodes.values_mut() {
        for (name, value) in mutated_seeds(node, mutation) {
            node.values.insert(name, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_move_on_by_the_mutation() {
        let catalogue = Catalogue::get();
        let spec = catalogue.node("Perlin").expect("Perlin is in the catalogue");
        let node = NodeData::new(spec, 1, 1234);
        let changes = mutated_seeds(&node, 3);
        assert_eq!(changes, vec![("Seed".to_string(), Value::from(1237))]);
    }

    #[test]
    fn mutation_wraps_like_dotnet_int() {
        let catalogue = Catalogue::get();
        let spec = catalogue.node("Perlin").expect("Perlin is in the catalogue");
        let node = NodeData::new(spec, 1, i32::MAX);
        let changes = mutated_seeds(&node, 1);
        assert_eq!(changes, vec![("Seed".to_string(), Value::from(i32::MIN))]);
    }
}
