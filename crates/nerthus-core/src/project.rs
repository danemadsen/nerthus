//! A terrain project (the `Terrain` document, saved as `.tor`).

use uuid::Uuid;

use crate::context::TerrainDefinition;
use crate::node::NodeInstance;
use crate::param::{Param, ParamList};

#[derive(Clone, Debug, PartialEq)]
pub struct BuildResolution {
    pub preview: i32,
    pub build: i32,
    pub resample: i32,
    pub tiled: i32,
    pub tiled_resample: i32,
    pub bake: i32,
    pub blend: i32,
}

impl Default for BuildResolution {
    fn default() -> Self {
        Self {
            preview: 512,
            build: 2048,
            resample: 2048,
            tiled: 2048,
            tiled_resample: 2048,
            bake: 1024,
            blend: 1,
        }
    }
}

/// Output value range (`Quantum`).
#[derive(Clone, Debug, PartialEq)]
pub struct OutputRange {
    pub range_type: i32,
    pub min: f64,
    pub max: f64,
    pub color_space: i32,
}

impl Default for OutputRange {
    fn default() -> Self {
        Self {
            range_type: 1,
            min: 0.0,
            max: 1.0,
            color_space: 1,
        }
    }
}

/// How tiled builds lay out their files (`OrganizeFiles`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum OrganizeFiles {
    None,
    #[default]
    NodeSubfolder,
    NodeAndPortSubfolder,
}

impl OrganizeFiles {
    pub fn xml_name(self) -> &'static str {
        match self {
            OrganizeFiles::None => "None",
            OrganizeFiles::NodeSubfolder => "NodeSubfolder",
            OrganizeFiles::NodeAndPortSubfolder => "NodeAndPortSubfolder",
        }
    }

    pub fn from_xml_name(name: &str) -> Self {
        match name {
            "None" => OrganizeFiles::None,
            "NodeAndPortSubfolder" => OrganizeFiles::NodeAndPortSubfolder,
            _ => OrganizeFiles::NodeSubfolder,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct BuildSettings {
    pub location: String,
    pub filename_format: String,
    /// 0 = normal build, 1 = tiled build.
    pub mode: i32,
    pub resolution: BuildResolution,
    pub tile_count: i32,
    pub overlap: i32,
    pub is_selective_build: bool,
    pub selected_tiles: Vec<String>,
    pub range: OutputRange,
    pub flip_y: bool,
    pub save_tor: bool,
    pub use_bakes: i32,
    pub organize_files: OrganizeFiles,
    pub preserve_tiled_cache: bool,
    pub custom_reference_image: Option<String>,
}

impl Default for BuildSettings {
    fn default() -> Self {
        Self {
            location: "[Builds]\\[Filename]\\[+++]\\".to_string(),
            filename_format: "[FileName]_y[y]_x[x]".to_string(),
            mode: 0,
            resolution: BuildResolution::default(),
            tile_count: 1,
            overlap: 0,
            is_selective_build: false,
            selected_tiles: Vec::new(),
            range: OutputRange::default(),
            flip_y: false,
            save_tor: false,
            use_bakes: 1,
            organize_files: OrganizeFiles::NodeSubfolder,
            preserve_tiled_cache: false,
            custom_reference_image: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ViewportPreferences {
    pub azimuth: f64,
    pub brightness: f64,
    pub heading: f64,
    pub water_level: f64,
    pub water: bool,
    pub material: i32,
    pub position: [f64; 3],
    pub rotation: [f64; 3],
}

impl Default for ViewportPreferences {
    fn default() -> Self {
        Self {
            azimuth: 33.0,
            brightness: 1.33,
            heading: 300.0,
            water_level: 0.0,
            water: false,
            material: 0,
            position: [0.0; 3],
            rotation: [0.0; 3],
        }
    }
}

/// Graph tab (`GraphDefinition`).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphTab {
    pub name: String,
    pub index: i32,
    pub order: i32,
    pub zoom: f64,
    pub pos_x: f64,
    pub pos_y: f64,
    pub selection_index: i32,
    pub is_pinned: bool,
    pub has_pinned: bool,
    pub has_pinned_color: bool,
    pub color: i32,
}

impl Default for GraphTab {
    fn default() -> Self {
        Self {
            name: "Graph".to_string(),
            index: 0,
            order: 1,
            zoom: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
            selection_index: -1,
            is_pinned: false,
            has_pinned: false,
            has_pinned_color: false,
            color: 0,
        }
    }
}

/// Sticky note on the graph canvas.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphNote {
    pub id: Uuid,
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub graph_index: i32,
}

/// Visual frame grouping nodes.
#[derive(Clone, Debug, PartialEq)]
pub struct GraphContainer {
    pub id: Uuid,
    pub title: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub color: i32,
    pub graph_index: i32,
}

/// Canvas view state for the main graph (`GraphData`).
#[derive(Clone, Debug, PartialEq)]
pub struct GraphView {
    pub zoom: f64,
    pub pos_x: f64,
    pub pos_y: f64,
    pub notes: Vec<GraphNote>,
    pub containers: Vec<GraphContainer>,
    /// Raw strings kept for formats we do not interpret yet.
    pub raw_notes: Vec<String>,
    pub raw_containers: Vec<String>,
}

impl Default for GraphView {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pos_x: 0.0,
            pos_y: 0.0,
            notes: Vec::new(),
            containers: Vec::new(),
            raw_notes: Vec::new(),
            raw_containers: Vec::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub file_format_version: String,
    pub workflow: String,
    pub has_resource: bool,
    pub prefix: Option<String>,
    pub export_format: i32,
    pub format: i32,
    pub resample: i32,
    pub baking_preference: i32,
    pub persist_cache: bool,
    pub use_central_cache: bool,
    pub infinity_tab_index: i32,
    pub build: BuildSettings,
    pub definition: TerrainDefinition,
    pub viewport: ViewportPreferences,
    pub lighting: ParamList,
    pub nodes: Vec<NodeInstance>,
    pub graph: GraphView,
    pub graphs: Vec<GraphTab>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "Untitled".to_string(),
            file_format_version: "1.3.0.0".to_string(),
            workflow: "Graph".to_string(),
            has_resource: false,
            prefix: None,
            export_format: 0,
            format: 1,
            resample: 0,
            baking_preference: 0,
            persist_cache: false,
            use_central_cache: true,
            infinity_tab_index: 0,
            build: BuildSettings::default(),
            definition: TerrainDefinition::default(),
            viewport: ViewportPreferences::default(),
            lighting: default_lighting(),
            nodes: Vec::new(),
            graph: GraphView::default(),
            graphs: vec![GraphTab::default()],
        }
    }
}

impl Project {
    pub fn node(&self, id: Uuid) -> Option<&NodeInstance> {
        self.nodes.iter().find(|node| node.id == id)
    }

    pub fn node_mut(&mut self, id: Uuid) -> Option<&mut NodeInstance> {
        self.nodes.iter_mut().find(|node| node.id == id)
    }

    pub fn node_index(&self, id: Uuid) -> Option<usize> {
        self.nodes.iter().position(|node| node.id == id)
    }

    /// Nodes consuming any output of `id`.
    pub fn dependents(&self, id: Uuid) -> Vec<Uuid> {
        let mut result = Vec::new();
        for node in &self.nodes {
            let consumes = node.ports.iter().any(|port| port.source.as_ref().is_some_and(|source| source.node == id));
            if consumes {
                result.push(node.id);
            }
        }
        result
    }

    /// Every node downstream of `id` (excluding itself).
    pub fn downstream(&self, id: Uuid) -> Vec<Uuid> {
        let mut visited = Vec::new();
        let mut stack = self.dependents(id);
        while let Some(next) = stack.pop() {
            if visited.contains(&next) {
                continue;
            }
            visited.push(next);
            stack.extend(self.dependents(next));
        }
        visited
    }

    /// Whether `upstream` feeds `node` (directly or indirectly).
    pub fn depends_on(&self, node: Uuid, upstream: Uuid) -> bool {
        let mut stack = vec![node];
        let mut visited = Vec::new();
        while let Some(current) = stack.pop() {
            if visited.contains(&current) {
                continue;
            }
            visited.push(current);
            let Some(instance) = self.node(current) else { continue };
            for port in &instance.ports {
                if let Some(source) = &port.source {
                    if source.node == upstream {
                        return true;
                    }
                    stack.push(source.node);
                }
            }
        }
        false
    }

    pub fn remove_node(&mut self, id: Uuid) {
        self.nodes.retain(|node| node.id != id);
        for node in &mut self.nodes {
            for port in &mut node.ports {
                if port.source.as_ref().is_some_and(|source| source.node == id) {
                    port.source = None;
                }
            }
        }
    }
}

/// Default renderer lighting block written into new projects.
pub fn default_lighting() -> ParamList {
    let params = vec![
        Param::double_plain("Sun azimuth", 48.0, 0.0, 360.0),
        Param::double_plain("Sun elevation", 30.0, 0.0, 90.0),
        Param::title("Atmosphere"),
        Param::double_plain("Exposure", 1.0, 0.0, 4.0),
        Param::double("Aerial Perspective", 0.25),
        Param::boolean("Shadows", true),
        Param::boolean("Ambient light", true),
        Param::boolean("Ambient occlusion", true),
        Param::title("Advanced"),
        Param::boolean("Auto Exposure", false),
        Param::double_plain("Sun intensity", 1.0, 0.0, 4.0),
        Param::double_plain("Ambient intensity", 1.0, 0.0, 4.0),
        Param::double_plain("Air density", 1.0, 0.0, 4.0),
        Param::double_plain("Haze", 1.0, 0.0, 4.0),
        Param::double_plain("Ozone layer thickness", 1.0, 0.0, 4.0),
        Param::title("Terrain Colors"),
        Param::double("Ground brightness", 0.65),
        Param::int("Index", 0, 0, 7),
        Param::int("IndexMask", 7, 0, 7),
        Param::boolean("Reverse", false),
        Param::boolean("Auto Level", true),
        Param::boolean("Grid", false),
        Param::boolean("Compass", false),
        Param::boolean("Skybox", false),
    ];
    ParamList::from_vec(params)
}
