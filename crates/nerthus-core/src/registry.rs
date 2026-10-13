//! Lookup table from `xsi:type` names to node behaviour.

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;

use crate::node::{NodeInstance, NodeKind, NodeSpec};
use crate::param::ParamList;

#[derive(Clone, Default)]
pub struct NodeRegistry {
    kinds: BTreeMap<&'static str, Arc<dyn NodeKind>>,
    /// Alternative names the same node had in earlier versions.
    aliases: BTreeMap<&'static str, &'static str>,
}

impl NodeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, kind: impl NodeKind + 'static) {
        let name = kind.type_name();
        self.kinds.insert(name, Arc::new(kind));
    }

    pub fn alias(&mut self, alias: &'static str, target: &'static str) {
        self.aliases.insert(alias, target);
    }

    pub fn get(&self, type_name: &str) -> Option<&Arc<dyn NodeKind>> {
        if let Some(kind) = self.kinds.get(type_name) {
            return Some(kind);
        }
        let target = self.aliases.get(type_name)?;
        self.kinds.get(target)
    }

    pub fn contains(&self, type_name: &str) -> bool {
        self.get(type_name).is_some()
    }

    pub fn kinds(&self) -> impl Iterator<Item = (&'static str, &Arc<dyn NodeKind>)> {
        self.kinds.iter().map(|(name, kind)| (*name, kind))
    }

    pub fn len(&self) -> usize {
        self.kinds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.kinds.is_empty()
    }

    pub fn spec(&self, type_name: &str) -> Option<NodeSpec> {
        let kind = self.get(type_name)?;
        let spec = kind.spec().finish();
        Some(spec)
    }

    /// Creates a fresh node instance with default parameters.
    pub fn instantiate(&self, type_name: &str) -> Option<NodeInstance> {
        let kind = self.get(type_name)?;
        let spec = kind.spec().finish();
        let mut instance = NodeInstance::from_spec(kind.type_name(), &spec);
        kind.refresh(&mut instance.params);
        Some(instance)
    }

    /// Rebuilds a node from its definition and applies saved values, keeping
    /// connections and identity (as deserializing does).
    pub fn rehydrate(&self, saved: &NodeInstance) -> Option<NodeInstance> {
        let kind = self.get(&saved.type_name)?;
        let spec = kind.spec().finish();
        let mut instance = NodeInstance::from_spec(kind.type_name(), &spec);
        instance.id = saved.id;
        // The deserialized node keeps its saved NodeType (only the fresh
        // node's parameters and ports are borrowed).
        instance.class = saved.class;
        instance.display_name = saved.display_name.clone();
        instance.file_name = saved.file_name.clone();
        instance.display_override = saved.display_override;
        instance.version = saved.version;
        instance.save_format = saved.save_format;
        instance.collapsed = saved.collapsed;
        instance.bypassed = saved.bypassed;
        instance.savable = saved.savable;
        instance.saving = saved.saving;
        instance.fave = saved.fave;
        instance.fave_index = saved.fave_index;
        instance.pinned = saved.pinned;
        instance.pinned_color = saved.pinned_color;
        instance.note = saved.note.clone();
        instance.graph_index = saved.graph_index;
        instance.position = saved.position;
        instance.extra_xml = saved.extra_xml.clone();
        kind.upgrade(saved, &mut instance);
        instance.params.apply_values_from(&saved.params);
        apply_mask_params(&mut instance.mask_params, &saved.mask_params);
        merge_ports(&mut instance.ports, &saved.ports);
        kind.refresh(&mut instance.params);
        Some(instance)
    }

    pub fn refresh_params(&self, type_name: &str, params: &mut ParamList) {
        if let Some(kind) = self.get(type_name) {
            kind.refresh(params);
        }
    }
}

fn apply_mask_params(target: &mut ParamList, saved: &ParamList) {
    if saved.is_empty() {
        return;
    }
    target.apply_values_from(saved);
}

/// Keeps the definition's port list and copies the saved state of ports
/// with the same name; saved ports the definition lacks are dropped, as in
/// the project loader (`GlobalUI`).
fn merge_ports(target: &mut [crate::port::Port], saved: &[crate::port::Port]) {
    for saved_port in saved {
        let Some(port) = target.iter_mut().find(|port| port.name == saved_port.name) else {
            continue;
        };
        port.save = saved_port.save;
        port.display_format = saved_port.display_format;
        port.order = saved_port.order;
        port.port_type = saved_port.port_type;
        port.optional = saved_port.optional;
        port.source = saved_port.source.clone();
        port.portal_state = saved_port.portal_state;
        port.note = saved_port.note.clone();
        if saved_port.display_name.is_some() {
            port.display_name = saved_port.display_name.clone();
        }
    }
}

/// Stable ordering helper used by toolboxes.
pub fn sorted_type_names(registry: &NodeRegistry) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = registry.kinds().map(|(name, _)| name).collect();
    names.sort_unstable();
    names
}

/// Generates a fresh id (re-exported so callers do not need `uuid`).
pub fn new_id() -> Uuid {
    Uuid::new_v4()
}
