//! Node ports. Connections live on input ports (`source`), like
//! `Port.SourceLayer` / `Port.SourcePort`.

use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PortType {
    In,
    Out,
    Mask,
    Param,
    Private,
}

impl PortType {
    pub fn xml_name(self) -> &'static str {
        match self {
            PortType::In => "In",
            PortType::Out => "Out",
            PortType::Mask => "Mask",
            PortType::Param => "Param",
            PortType::Private => "Private",
        }
    }

    pub fn from_xml_name(name: &str) -> Self {
        match name {
            "Out" => PortType::Out,
            "Mask" => PortType::Mask,
            "Param" => PortType::Param,
            "Private" => PortType::Private,
            _ => PortType::In,
        }
    }

    pub fn is_input(self) -> bool {
        matches!(self, PortType::In | PortType::Mask)
    }
}

/// How a port's data should be previewed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum DisplayFormat {
    #[default]
    Terrain,
    Mask,
    Bitmap,
    Default,
    Any,
}

impl DisplayFormat {
    pub fn xml_name(self) -> &'static str {
        match self {
            DisplayFormat::Terrain => "Terrain",
            DisplayFormat::Mask => "Mask",
            DisplayFormat::Bitmap => "Bitmap",
            DisplayFormat::Default => "Default",
            DisplayFormat::Any => "Any",
        }
    }

    pub fn from_xml_name(name: &str) -> Self {
        match name {
            "Mask" => DisplayFormat::Mask,
            "Bitmap" => DisplayFormat::Bitmap,
            "Default" => DisplayFormat::Default,
            "Any" => DisplayFormat::Any,
            _ => DisplayFormat::Terrain,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum PortalState {
    #[default]
    None,
    Publishing,
    Consuming,
}

impl PortalState {
    pub fn xml_name(self) -> &'static str {
        match self {
            PortalState::None => "None",
            PortalState::Publishing => "Publishing",
            PortalState::Consuming => "Consuming",
        }
    }

    pub fn from_xml_name(name: &str) -> Self {
        match name {
            "Publishing" => PortalState::Publishing,
            "Consuming" => PortalState::Consuming,
            _ => PortalState::None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PortSource {
    pub node: Uuid,
    pub port: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Port {
    pub name: String,
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub order: i32,
    pub port_type: PortType,
    pub display_format: DisplayFormat,
    pub optional: bool,
    pub save: bool,
    pub portal_state: PortalState,
    pub source: Option<PortSource>,
}

impl Port {
    pub fn new(name: &str, order: i32, port_type: PortType, optional: bool) -> Self {
        Self {
            name: name.to_string(),
            display_name: None,
            note: None,
            order,
            port_type,
            display_format: DisplayFormat::Terrain,
            optional,
            save: false,
            portal_state: PortalState::None,
            source: None,
        }
    }

    pub fn input(name: &str, order: i32) -> Self {
        Self::new(name, order, PortType::In, false)
    }

    pub fn optional_input(name: &str, order: i32) -> Self {
        Self::new(name, order, PortType::In, true)
    }

    pub fn output(name: &str, order: i32) -> Self {
        Self::new(name, order, PortType::Out, true)
    }

    pub fn mask() -> Self {
        Self::new("Mask", 99, PortType::Mask, true)
    }

    pub fn with_format(mut self, format: DisplayFormat) -> Self {
        self.display_format = format;
        self
    }

    pub fn with_display_name(mut self, display_name: &str) -> Self {
        self.display_name = Some(display_name.to_string());
        self
    }

    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }

    pub fn is_input(&self) -> bool {
        self.port_type.is_input()
    }

    pub fn is_output(&self) -> bool {
        self.port_type == PortType::Out
    }

    pub fn is_connected(&self) -> bool {
        self.source.is_some()
    }
}

/// Helpers matching the `PortList` conveniences.
pub struct PortSet;

impl PortSet {
    /// The standard ports: Input, (Mask), Output.
    pub fn standard(with_mask: bool) -> Vec<Port> {
        let mut ports = vec![Port::input("Input", 0)];
        if with_mask {
            ports.push(Port::mask());
        }
        ports.push(Port::output("Output", 0));
        ports
    }

    /// A generator's ports: Output, (Mask).
    pub fn generator(with_mask: bool) -> Vec<Port> {
        let mut ports = vec![Port::output("Output", 0)];
        if with_mask {
            ports.push(Port::mask());
        }
        ports
    }
}

pub fn find_port<'a>(ports: &'a [Port], name: &str) -> Option<&'a Port> {
    ports.iter().find(|port| port.name == name)
}

pub fn find_port_mut<'a>(ports: &'a mut [Port], name: &str) -> Option<&'a mut Port> {
    ports.iter_mut().find(|port| port.name == name)
}
