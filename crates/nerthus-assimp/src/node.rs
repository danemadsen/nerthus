//! `aiNode`, as far as importers that build meshes from the node graph
//! need it: nodes in one vector, linked by index.

use crate::matrix::Matrix4;

#[derive(Clone)]
pub(crate) struct Node {
    pub name: Vec<u8>,
    pub transform: Matrix4,
    pub parent: Option<usize>,
    pub children: Vec<usize>,
}

impl Node {
    pub fn new(name: Vec<u8>, transform: Matrix4, parent: Option<usize>) -> Self {
        Self { name, transform, parent, children: Vec::new() }
    }
}
