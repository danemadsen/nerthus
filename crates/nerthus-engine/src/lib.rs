//! The engine model: maps, the node catalogue, projects and graph
//! evaluation.

pub mod automation;
pub mod catalogue;
pub mod editing;
pub mod format_names;
pub mod fragment;
pub mod graph;
pub mod humanize;
pub mod jace;
pub mod json;
mod loops;
pub mod macros;
pub mod migrate;
pub mod map;
pub mod mutation;
pub mod paths;
pub mod project;
pub mod region;
pub mod writer;

pub use catalogue::{Catalogue, ModifierSpec, NodeSpec, ParameterSpec, PortSpec, ValueType};
pub use graph::{check_required_ports, ApplicationMode, BuildContext, BuildResult, Cache, Engine, EnginePreferences, ModifierBuilder, NodeBuilder, NodeError, NodeState, PortIntents, Registry, StateOverride};
pub use map::{Map, MapInfo, RenderIntent, Resolution};
pub use project::{Asset, ConnectionRecord, ModifierData, NodeData, PortData, Project, ProjectError, Terrain};
pub use region::Region;
pub use writer::{to_fragment_json, to_macro_asset_json, to_terrain_json};
