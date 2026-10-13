//! Node definitions (what a node type looks like) and node instances (a
//! node placed in a project).

use std::sync::Arc;

use uuid::Uuid;

use crate::context::BuildContext;
use crate::field::Field;
use crate::param::{Param, ParamList};
use crate::port::{DisplayFormat, Port, PortType};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeClass {
    Primitive,
    Maker,
    Filter,
    Adjustment,
    Erosion,
    Combiner,
    Input,
    Output,
    Color,
    Preview,
    Selector,
    DataMap,
    Portal,
    Logic,
    Agnostic,
    Snow,
    Water,
    LookDev,
    Render,
    Arboreal,
}

impl NodeClass {
    pub const ALL: [NodeClass; 20] = [
        NodeClass::Primitive,
        NodeClass::Maker,
        NodeClass::Filter,
        NodeClass::Adjustment,
        NodeClass::Erosion,
        NodeClass::Combiner,
        NodeClass::Input,
        NodeClass::Output,
        NodeClass::Color,
        NodeClass::Preview,
        NodeClass::Selector,
        NodeClass::DataMap,
        NodeClass::Portal,
        NodeClass::Logic,
        NodeClass::Agnostic,
        NodeClass::Snow,
        NodeClass::Water,
        NodeClass::LookDev,
        NodeClass::Render,
        NodeClass::Arboreal,
    ];

    pub fn xml_name(self) -> &'static str {
        match self {
            NodeClass::Primitive => "Primitive",
            NodeClass::Maker => "Maker",
            NodeClass::Filter => "Filter",
            NodeClass::Adjustment => "Adjustment",
            NodeClass::Erosion => "Erosion",
            NodeClass::Combiner => "Combiner",
            NodeClass::Input => "Input",
            NodeClass::Output => "Output",
            NodeClass::Color => "Color",
            NodeClass::Preview => "Preview",
            NodeClass::Selector => "Selector",
            NodeClass::DataMap => "DataMap",
            NodeClass::Portal => "Portal",
            NodeClass::Logic => "Logic",
            NodeClass::Agnostic => "Agnostic",
            NodeClass::Snow => "Snow",
            NodeClass::Water => "Water",
            NodeClass::LookDev => "LookDev",
            NodeClass::Render => "Render",
            NodeClass::Arboreal => "Arboreal",
        }
    }

    pub fn from_xml_name(name: &str) -> Option<Self> {
        NodeClass::ALL.iter().copied().find(|class| class.xml_name() == name)
    }

    /// Accent colour used for node headers and connectors (the theme's).
    pub fn accent_color(self) -> [u8; 3] {
        match self {
            NodeClass::Primitive | NodeClass::Maker | NodeClass::Input => [0x8e, 0xa8, 0x3a],
            NodeClass::Filter => [0x3a, 0x8e, 0xa8],
            NodeClass::Adjustment | NodeClass::Combiner => [0x9a, 0x9a, 0x9a],
            NodeClass::Erosion => [0xc0, 0x7a, 0x3a],
            NodeClass::Output => [0xd0, 0x40, 0x40],
            NodeClass::Color | NodeClass::Render => [0xc8, 0x4a, 0xc8],
            NodeClass::Selector | NodeClass::DataMap => [0x4a, 0x7a, 0xd8],
            NodeClass::Portal | NodeClass::Logic => [0xd8, 0xc0, 0x4a],
            NodeClass::Snow => [0xd8, 0xe8, 0xf0],
            NodeClass::Water => [0x3a, 0x6a, 0xd8],
            NodeClass::LookDev | NodeClass::Arboreal => [0x6a, 0xb8, 0x7a],
            NodeClass::Preview | NodeClass::Agnostic => [0x80, 0x80, 0x80],
        }
    }
}

/// How a node reacts to a click in the 3D viewport (`PickerMode`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PickerMode {
    #[default]
    None,
    /// Samples a height under the cursor.
    Height,
    /// Places the node's effect at the cursor.
    Placement,
    Other,
}

/// The custom property editor a node opens in the UI (`Editor`, the
/// editor's class name without its namespace).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum NodeEditor {
    #[default]
    None,
    Cluter,
    Draw,
    FileInput,
    Fx,
    Island,
    Light,
    Mask,
    Mesher,
    Output,
    Resource,
    SatelliteMap,
    Seamless,
    Stroke,
    Synth,
    Vector,
    Vegetation,
}

/// A viewport pick handed to `NodeKind::pick`.
pub struct PickContext<'a> {
    /// Horizontal position on the terrain, 0 at the left edge to 1 at the right.
    pub x: f32,
    /// Vertical position on the terrain, 0 at the top edge to 1 at the bottom.
    pub y: f32,
    /// Resolution of the cached preview data.
    pub resolution: usize,
    /// The node's cached `Output` data, if any.
    pub output: Option<&'a Field>,
    /// The cached data feeding the node's first input, if any.
    pub input: Option<&'a Field>,
    /// The parameter the user edited last (`LayerBase.LastModified`).
    pub last_modified: Option<&'a str>,
}

impl PickContext<'_> {
    /// The lookup `Data[(int)(y * U * U + x * U)]`, including the way the
    /// fractional row leaks into the column.
    pub fn sample(&self, field: &Field) -> Option<f32> {
        let resolution = self.resolution as f32;
        let column = self.x * resolution;
        let row = self.y * resolution;
        let index = (row * resolution + column) as usize;
        field.data.get(index).copied()
    }
}

/// Static description of a node type, built by `NodeKind::spec`.
#[derive(Clone, Debug)]
pub struct NodeSpec {
    /// `Name`, also the default display name.
    pub name: String,
    pub class: NodeClass,
    pub params: ParamList,
    pub ports: Vec<Port>,
    /// Whether the generic post-process (mask) parameters apply.
    pub can_mask: bool,
    pub has_intrinsic_params: bool,
    pub is_cancellable: bool,
    pub is_heavy: bool,
    pub cannot_tile: bool,
    pub picker: PickerMode,
    /// Toolbox sub-menu (`Variant`): 1 lists the node in the second
    /// menu of its class.
    pub variant: i32,
    /// Kept out of the toolbox and search (`IsHidden`).
    pub is_hidden: bool,
    /// Internal node, also kept out of the toolbox (`IsPrivate`).
    pub is_private: bool,
    /// New instances start collapsed (`Collapsed`).
    pub collapsed: bool,
    /// Glyph drawn on the node instead of an icon (`IconText`).
    pub icon_text: String,
    pub node_size: i32,
    pub keywords: String,
    pub description: String,
    pub version: i32,
    /// Display format new instances start with (`DisplayOverride`).
    pub display_override: DisplayFormat,
    pub editor: NodeEditor,
    /// The output drawn over the terrain in the viewport, such as a water
    /// node's water mask.
    pub overlay_port: Option<String>,
}

impl NodeSpec {
    pub fn new(name: &str, class: NodeClass) -> Self {
        Self {
            name: name.to_string(),
            class,
            params: ParamList::new(),
            ports: Vec::new(),
            can_mask: true,
            has_intrinsic_params: true,
            is_cancellable: false,
            is_heavy: false,
            cannot_tile: class == NodeClass::Primitive,
            picker: PickerMode::None,
            variant: 0,
            is_hidden: false,
            is_private: false,
            collapsed: false,
            icon_text: String::new(),
            node_size: 1,
            keywords: String::new(),
            description: String::new(),
            version: 1,
            display_override: DisplayFormat::Default,
            editor: NodeEditor::None,
            overlay_port: None,
        }
    }

    pub fn param(mut self, param: Param) -> Self {
        self.params.push(param);
        self
    }

    pub fn params(mut self, params: impl IntoIterator<Item = Param>) -> Self {
        self.params.extend(params);
        self
    }

    pub fn port(mut self, port: Port) -> Self {
        self.ports.push(port);
        self
    }

    /// Applies the node defaults: standard ports when none are
    /// declared, and display formats for colour / data-map nodes.
    pub fn finish(mut self) -> Self {
        if self.ports.is_empty() {
            let with_mask = self.class != NodeClass::Color;
            self.ports = crate::port::PortSet::standard(with_mask);
        }
        if self.class == NodeClass::Color {
            for port in self.ports.iter_mut().filter(|port| port.port_type == PortType::Out) {
                port.display_format = DisplayFormat::Bitmap;
            }
        }
        if self.class == NodeClass::DataMap {
            for port in self.ports.iter_mut().filter(|port| port.port_type == PortType::Out) {
                port.display_format = DisplayFormat::Mask;
            }
        }
        self
    }

    pub fn mask_params(&self) -> ParamList {
        if self.can_mask {
            crate::postprocess_params::post_process_params()
        } else {
            ParamList::new()
        }
    }
}

/// Error raised by node processing.
#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("cancelled")]
    Cancelled,
    #[error("port {0} needs to be connected")]
    MissingInput(String),
    #[error("{0}")]
    Failed(String),
}

impl NodeError {
    pub fn failed(message: impl Into<String>) -> Self {
        NodeError::Failed(message.into())
    }
}

/// One connected input handed to a node, in port order.
#[derive(Clone, Debug)]
pub struct NodeInput {
    pub port: String,
    pub field: Arc<Field>,
}

/// One output produced by a node.
#[derive(Clone, Debug)]
pub struct NodeOutput {
    pub port: String,
    pub field: Field,
    /// Field the mask blends towards (the node's primary input), if any.
    pub base: Option<Arc<Field>>,
    /// Mask to blend with; `None` means no masking for this output.
    pub mask: Option<Arc<Field>>,
    /// Whether the generic post-process should run (only for "Output").
    pub post_process: bool,
}

impl NodeOutput {
    /// An output without base or mask.
    pub fn plain(port: &str, field: Field) -> Self {
        Self {
            port: port.to_string(),
            field,
            base: None,
            mask: None,
            post_process: port == "Output",
        }
    }
}

/// What a node sees while it runs.
pub struct NodeContext<'a> {
    pub build: &'a BuildContext,
    pub node_id: Uuid,
    pub class: NodeClass,
    /// Serialized node version (behaviour changed across releases).
    pub version: i32,
    pub params: &'a ParamList,
    pub inputs: Vec<NodeInput>,
    pub mask: Option<Arc<Field>>,
    pub outputs: Vec<NodeOutput>,
    pub warnings: Vec<String>,
    /// Shared state the node may stash between runs (e.g. a loaded file).
    pub extra: &'a crate::context::NodeState,
}

impl<'a> NodeContext<'a> {
    pub fn size(&self) -> usize {
        self.build.size
    }

    pub fn pixel_count(&self) -> usize {
        self.build.size * self.build.size
    }

    pub fn is_cancelled(&self) -> bool {
        self.build.is_cancelled()
    }

    pub fn check_cancelled(&self) -> Result<(), NodeError> {
        if self.build.is_cancelled() {
            return Err(NodeError::Cancelled);
        }
        Ok(())
    }

    pub fn progress(&self, fraction: f64) {
        self.build.report_progress(fraction);
    }

    /// `S[index]`: the index-th *connected* input.
    pub fn input(&self, index: usize) -> Option<&Arc<Field>> {
        self.inputs.get(index).map(|input| &input.field)
    }

    /// The connected input on `port`.
    pub fn input_named(&self, port: &str) -> Option<&Arc<Field>> {
        self.inputs.iter().find(|input| input.port == port).map(|input| &input.field)
    }

    pub fn require_input(&self, index: usize) -> Result<&Arc<Field>, NodeError> {
        self.input(index).ok_or_else(|| NodeError::MissingInput("Input".into()))
    }

    pub fn input_count(&self) -> usize {
        self.inputs.len()
    }

    /// The main output, masked and post-processed over the first input.
    pub fn set_output(&mut self, field: Field) {
        let base = self.input(0).cloned();
        let mask = self.mask.clone();
        self.push_output("Output", field, base, mask, true);
    }

    /// Main output blended towards an explicit base.
    pub fn set_output_with_base(&mut self, field: Field, base: Option<Arc<Field>>) {
        let mask = self.mask.clone();
        self.push_output("Output", field, base, mask, true);
    }

    /// Secondary output without masking.
    pub fn set_named_output(&mut self, port: &str, field: Field) {
        self.push_output(port, field, None, None, false);
    }

    /// Secondary output that still honours the node mask.
    pub fn set_named_output_masked(&mut self, port: &str, field: Field, base: Option<Arc<Field>>) {
        let mask = self.mask.clone();
        self.push_output(port, field, base, mask, false);
    }

    pub fn push_output(&mut self, port: &str, field: Field, base: Option<Arc<Field>>, mask: Option<Arc<Field>>, post_process: bool) {
        let post_process = post_process && port == "Output";
        self.outputs.push(NodeOutput {
            port: port.to_string(),
            field,
            base,
            mask,
            post_process,
        });
    }

    pub fn warn(&mut self, message: impl Into<String>) {
        self.warnings.push(message.into());
    }
}

/// Behaviour of one node type.
pub trait NodeKind: Send + Sync {
    /// The XML `xsi:type` name (also the registry key).
    fn type_name(&self) -> &'static str;

    /// Parameters, ports and flags with default values.
    fn spec(&self) -> NodeSpec;

    /// Updates parameter visibility after an edit.
    fn refresh(&self, _params: &mut ParamList) {}

    /// Runs the node.
    fn process(&self, ctx: &mut NodeContext) -> Result<(), NodeError>;

    /// Adjusts a freshly built node before saved values are copied onto it
    /// when loading an older file (the load-time special cases).
    fn upgrade(&self, _saved: &NodeInstance, _fresh: &mut NodeInstance) {}

    /// Applies a viewport pick to the parameters; only called
    /// for nodes whose spec has a `picker` mode.
    fn pick(&self, _params: &mut ParamList, _pick: &PickContext) {}

    /// Category shown in the toolbox; defaults to the node class.
    fn category(&self) -> &'static str {
        ""
    }
}

/// A node placed in a project, with its serialized state.
#[derive(Clone, Debug)]
pub struct NodeInstance {
    pub id: Uuid,
    pub type_name: String,
    pub name: String,
    pub class: NodeClass,
    pub display_name: Option<String>,
    pub file_name: Option<String>,
    pub params: ParamList,
    pub mask_params: ParamList,
    pub ports: Vec<Port>,
    pub display_override: DisplayFormat,
    pub version: i32,
    pub save_format: i32,
    pub collapsed: bool,
    pub bypassed: bool,
    pub savable: bool,
    pub saving: bool,
    pub fave: bool,
    pub fave_index: i32,
    pub pinned: bool,
    pub pinned_color: bool,
    pub note: Option<String>,
    pub graph_index: i32,
    pub position: (f64, f64),
    pub baked: bool,
    /// Unknown child elements from the file, re-emitted verbatim.
    pub extra_xml: Vec<String>,
}

impl NodeInstance {
    pub fn from_spec(type_name: &str, spec: &NodeSpec) -> Self {
        Self {
            id: Uuid::new_v4(),
            type_name: type_name.to_string(),
            name: spec.name.clone(),
            class: spec.class,
            display_name: None,
            file_name: None,
            params: spec.params.clone(),
            mask_params: spec.mask_params(),
            ports: spec.ports.clone(),
            display_override: spec.display_override,
            version: spec.version,
            save_format: -1,
            collapsed: spec.collapsed,
            bypassed: false,
            savable: false,
            saving: false,
            fave: false,
            fave_index: 0,
            pinned: false,
            pinned_color: false,
            note: None,
            graph_index: 0,
            position: (0.0, 0.0),
            baked: false,
            extra_xml: Vec::new(),
        }
    }

    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    /// The file name builds save this node under (`FileName`, which
    /// falls back to the type's name, not the display name).
    pub fn file_label(&self) -> &str {
        self.file_name.as_deref().unwrap_or(&self.name)
    }

    /// Marks the node for saving in builds: every output port is
    /// saved and the file name follows the display name.
    pub fn set_saving(&mut self, saving: bool) {
        self.savable = saving;
        if saving {
            self.saving = true;
        }
        for port in self.ports.iter_mut().filter(|port| port.is_output()) {
            port.save = saving;
        }
        if saving {
            self.file_name = Some(file_safe_name(self.label()));
        }
    }

    /// Whether a build writes this node's marked ports.
    pub fn saves_in_build(&self) -> bool {
        self.savable && self.saving && !self.bypassed
    }

    pub fn port(&self, name: &str) -> Option<&Port> {
        self.ports.iter().find(|port| port.name == name)
    }

    pub fn port_mut(&mut self, name: &str) -> Option<&mut Port> {
        self.ports.iter_mut().find(|port| port.name == name)
    }

    pub fn input_ports(&self) -> impl Iterator<Item = &Port> {
        self.ports.iter().filter(|port| port.is_input())
    }

    pub fn output_ports(&self) -> impl Iterator<Item = &Port> {
        self.ports.iter().filter(|port| port.is_output())
    }

    /// Nodes whose output is shown as a mask/bitmap.
    pub fn is_data_display(&self) -> bool {
        if matches!(self.display_override, DisplayFormat::Mask | DisplayFormat::Bitmap) {
            return true;
        }
        matches!(self.class, NodeClass::DataMap | NodeClass::Selector | NodeClass::Color)
    }

    /// Cache key for the node's own parameters.
    pub fn hash_string(&self) -> String {
        let mut text = self.id.simple().to_string();
        text.push_str(&self.params.hash_string());
        if !self.mask_params.is_empty() {
            for param in &self.mask_params {
                text.push_str(&param.hash_fragment());
            }
        }
        if self.bypassed {
            text.push_str("|bypassed");
        }
        text
    }
}

/// Replaces the characters Windows forbids in file names
/// (`Path.GetInvalidFileNameChars`) with underscores.
pub fn file_safe_name(name: &str) -> String {
    let is_invalid = |character: char| matches!(character, '"' | '<' | '>' | '|' | ':' | '*' | '?' | '\\' | '/') || (character as u32) < 32;
    name.chars().map(|character| if is_invalid(character) { '_' } else { character }).collect()
}
