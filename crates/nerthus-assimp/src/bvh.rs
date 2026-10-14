//! Assimp's BVH motion capture importer (`BVHLoader.cpp`): the HIERARCHY
//! (a ROOT, JOINTs and End Sites with offsets and channels) and the
//! MOTION values, all whitespace-separated tokens. The mesh is
//! `SkeletonMeshBuilder`'s over the offsets; building the animation after
//! it can still fail (a node of six channels without all three position
//! channels).
//!
//! A node's CHANNELS after a child of it go through a reference into the
//! node list taken before the child was added: once the list has grown
//! (MSVC's growth) since, that is a write to freed memory, undefined
//! behaviour, an error here.

use crate::cast::to_unsigned;
use crate::fast_atof::fast_atoreal_move;
use crate::matrix::Matrix4;
use crate::msvc;
use crate::node::Node;
use crate::scene::{ImportError, Scene, check_allocation, fail};
use crate::skeleton;

/// Nesting the C++ recursion would only end with a stack overflow.
const MAX_DEPTH: usize = 2048;

/// `sizeof(aiVectorKey)` and `sizeof(aiQuatKey)`.
const KEY_SIZE: u64 = 24;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Channel {
    PositionX,
    PositionY,
    PositionZ,
    RotationX,
    RotationY,
    RotationZ,
}

/// `BVHLoader::Node`: a joint's channels (End Sites have none).
struct Joint {
    channels: Vec<Channel>,
    /// The node list's reallocations when the joint was added.
    reallocations: usize,
}

struct Loader<'a> {
    buffer: &'a [u8],
    position: usize,
    graph: Vec<Node>,
    joints: Vec<Joint>,
    joint_capacity: usize,
    reallocations: usize,
}

/// `isspace` in the C locale.
fn is_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

impl<'a> Loader<'a> {
    /// `GetNextToken`: up to whitespace; a lone brace is a token.
    fn next_token(&mut self) -> &'a [u8] {
        while self.position < self.buffer.len() && is_space(self.buffer[self.position]) {
            self.position += 1;
        }
        let start = self.position;
        while self.position < self.buffer.len() && !is_space(self.buffer[self.position]) {
            self.position += 1;
            let token = &self.buffer[start..self.position];
            if token == b"{" || token == b"}" {
                break;
            }
        }
        &self.buffer[start..self.position]
    }

    /// `GetNextTokenAsFloat`: the whole token must be the number.
    fn next_float(&mut self) -> Result<f32, ImportError> {
        let token = self.next_token();
        if token.is_empty() {
            return fail("Unexpected end of file while trying to read a float");
        }
        let (value, end) = fast_atoreal_move(token, 0, true)?;
        if end != token.len() {
            return fail("Expected a floating point number");
        }
        Ok(value)
    }

    fn expect(&mut self, expected: &[u8], message: &str) -> Result<(), ImportError> {
        if self.next_token() != expected {
            return fail(message);
        }
        Ok(())
    }

    /// `ReadNodeOffset`: the node's translation.
    fn read_offset(&mut self, node: usize) -> Result<(), ImportError> {
        let x = self.next_float()?;
        let y = self.next_float()?;
        let z = self.next_float()?;
        self.graph[node].transform = Matrix4::translation([x, y, z]);
        Ok(())
    }

    /// `ReadNodeChannels` for the joint at `joint`.
    fn read_channels(&mut self, joint: usize) -> Result<(), ImportError> {
        let count = to_unsigned(self.next_float()? as f64);
        for _ in 0..count {
            let channel = match self.next_token() {
                b"Xposition" => Channel::PositionX,
                b"Yposition" => Channel::PositionY,
                b"Zposition" => Channel::PositionZ,
                b"Xrotation" => Channel::RotationX,
                b"Yrotation" => Channel::RotationY,
                b"Zrotation" => Channel::RotationZ,
                _ => return fail("Invalid channel specifier"),
            };
            if self.joints[joint].reallocations != self.reallocations {
                return fail("undefined behaviour: channels written to a freed node");
            }
            self.joints[joint].channels.push(channel);
        }
        Ok(())
    }

    fn add_graph_node(&mut self, name: &[u8], parent: Option<usize>) -> usize {
        let index = self.graph.len();
        self.graph.push(Node::new(name.to_vec(), Matrix4::IDENTITY, parent));
        if let Some(parent) = parent {
            self.graph[parent].children.push(index);
        }
        index
    }

    /// `ReadNode`.
    fn read_node(&mut self, parent: Option<usize>, depth: usize) -> Result<(), ImportError> {
        if depth > MAX_DEPTH {
            return fail("BVH: nodes nested too deep");
        }
        let name = self.next_token();
        if name.is_empty() || name == b"{" {
            return fail("Expected node name");
        }
        self.expect(b"{", "Expected opening brace")?;
        // The node joins its parent's children once read; children are
        // numbered in reading order either way.
        let node = self.add_graph_node(name, parent);
        if self.joints.len() == self.joint_capacity {
            self.joint_capacity = msvc::vector_growth(self.joint_capacity, self.joints.len() + 1);
            self.reallocations += 1;
        }
        let joint = self.joints.len();
        self.joints.push(Joint { channels: Vec::new(), reallocations: self.reallocations });

        loop {
            let token = self.next_token();
            match token {
                b"OFFSET" => self.read_offset(node)?,
                b"CHANNELS" => self.read_channels(joint)?,
                b"JOINT" => self.read_node(Some(node), depth + 1)?,
                b"End" => {
                    self.expect(b"Site", "Expected \"End Site\" keyword")?;
                    self.read_end_site(node)?;
                }
                b"}" => break,
                _ => return fail("Unknown keyword"),
            }
        }
        Ok(())
    }

    /// `ReadEndSite`.
    fn read_end_site(&mut self, parent: usize) -> Result<(), ImportError> {
        self.expect(b"{", "Expected opening brace")?;
        let node = self.add_graph_node(b"EndSite", Some(parent));
        loop {
            match self.next_token() {
                b"OFFSET" => self.read_offset(node)?,
                b"}" => break,
                _ => return fail("Unknown keyword"),
            }
        }
        Ok(())
    }

    /// `ReadMotion`: the frame count and every frame's values.
    fn read_motion(&mut self) -> Result<u32, ImportError> {
        self.expect(b"Frames:", "Expected frame count \"Frames:\"")?;
        let frame_count = to_unsigned(self.next_float()? as f64);
        let frame = self.next_token();
        let time = self.next_token();
        if frame != b"Frame" || time != b"Time:" {
            return fail("Expected frame duration \"Frame Time:\"");
        }
        self.next_float()?;
        for joint in &self.joints {
            check_allocation(joint.channels.len() as u64 * frame_count as u64, 4)?;
        }
        let values_per_frame: usize = self.joints.iter().map(|joint| joint.channels.len()).sum();
        if values_per_frame > 0 {
            for _ in 0..frame_count as u64 * values_per_frame as u64 {
                self.next_float()?;
            }
        }
        Ok(frame_count)
    }
}

pub fn read(bytes: &[u8]) -> Result<Scene, ImportError> {
    if bytes.is_empty() {
        return fail("File is too small.");
    }
    let mut loader = Loader { buffer: bytes, position: 0, graph: Vec::new(), joints: Vec::new(), joint_capacity: 0, reallocations: 0 };
    loader.expect(b"HIERARCHY", "Expected header string \"HIERARCHY\".")?;
    loader.expect(b"ROOT", "Expected root node \"ROOT\".")?;
    loader.read_node(None, 0)?;
    loader.expect(b"MOTION", "Expected beginning of motion data \"MOTION\".")?;
    let frame_count = loader.read_motion()?;

    let mesh = skeleton::build_mesh(&loader.graph, 0, false);

    // `CreateAnimation`, for its failures.
    for joint in &loader.joints {
        let has_positions = joint.channels.len() == 6;
        if has_positions {
            check_allocation(frame_count as u64, KEY_SIZE)?;
            let complete = [Channel::PositionX, Channel::PositionY, Channel::PositionZ].iter().all(|channel| joint.channels.contains(channel));
            if frame_count > 0 && !complete {
                return fail("Missing position channel in node");
            }
        }
        check_allocation(frame_count as u64, KEY_SIZE)?;
    }
    Ok(Scene { meshes: vec![mesh] })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SKELETON: &[u8] = b"HIERARCHY\nROOT Hips\n{\n\tOFFSET 0 0 0\n\tCHANNELS 6 Xposition Yposition Zposition Zrotation Xrotation Yrotation\n\
        \tJOINT Chest\n\t{\n\t\tOFFSET 0 5 0\n\t\tCHANNELS 3 Zrotation Xrotation Yrotation\n\t\tEnd Site\n\t\t{\n\t\t\tOFFSET 0 3 0\n\t\t}\n\t}\n}\n\
        MOTION\nFrames: 2\nFrame Time: 0.033\n0 0 0 0 0 0 0 0 0\n1 1 1 1 1 1 1 1 1\n";

    #[test]
    fn a_chain_becomes_pointers_and_a_knob() {
        let scene = read(SKELETON).unwrap();
        // A pointer from Hips to Chest, one from Chest to the End Site, a
        // knob at the End Site: 12 + 12 + 24 vertices.
        assert_eq!(scene.meshes[0].vertex_count, 48);
    }

    #[test]
    fn six_channels_need_three_positions() {
        let text = String::from_utf8(SKELETON.to_vec()).unwrap().replace("Xposition Yposition", "Xrotation Yposition");
        assert!(read(text.as_bytes()).is_err());
    }
}
