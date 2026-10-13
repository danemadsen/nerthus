//! Core data model shared by every Nerthus crate.
//!
//! The model mirrors the engine's so projects round-trip: a [`Project`]
//! holds [`NodeInstance`]s whose behaviour is provided by [`NodeKind`]
//! implementations looked up in a [`NodeRegistry`].

pub mod context;
pub mod dotnet_format;
pub mod dotnet_random;
pub mod field;
pub mod node;
pub mod param;
pub mod port;
pub mod postprocess_params;
pub mod project;
pub mod registry;
pub mod slider;

pub use context::{BuildContext, CacheLevel, NodeState, ResourceProvider, TerrainDefinition, TileInfo};
pub use dotnet_random::DotNetRandom;
pub use field::Field;
pub use node::{file_safe_name, NodeClass, NodeContext, NodeEditor, NodeError, NodeInput, NodeInstance, NodeKind, NodeOutput, NodeSpec, PickContext, PickerMode};
pub use param::{CurvePoint, DisplayType, Param, ParamList, ParamType, ParamValue, Rgba, Stroke, StringKind};
pub use port::{DisplayFormat, Port, PortSet, PortSource, PortType, PortalState};
pub use postprocess_params::{post_process_params, random_seed, seed_param, seed_param_named};
pub use project::{BuildSettings, GraphTab, GraphView, Project, ViewportPreferences};
pub use registry::NodeRegistry;
pub use uuid::Uuid;
