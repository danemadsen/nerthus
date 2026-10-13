//! obj-dump <file>: the mesh Nerthus's port of `MeshPort_Load` builds, in
//! the format of the native shim's obj-dump
//! (positions as (x, z, y) hex bits, every triangle's indices; "failed"
//! when there is no mesh).

use nerthus_nodes::{mesh_port, obj_loader};

fn main() {
    let path = std::env::args().nth(1).expect("usage: obj-dump <file>");
    let extension = path.rfind('.').map(|dot| &path[dot + 1..]).unwrap_or("");
    let mesh = if extension == "obj" {
        let bytes = std::fs::read(&path).expect("cannot read the file");
        obj_loader::parse_obj(&bytes).map(|model| obj_loader::mesh_from_model(&model))
    } else {
        if let Err(error) = nerthus_assimp::read_file(&path) {
            eprintln!("{error}");
        }
        mesh_port::load_assimp(&path)
    };
    let Some(mesh) = mesh else {
        println!("failed");
        return;
    };
    println!("vertices {}", mesh.vertices.len());
    for vertex in &mesh.vertices {
        println!("{:08x} {:08x} {:08x}", vertex[0].to_bits(), vertex[1].to_bits(), vertex[2].to_bits());
    }
    println!("triangles {}", mesh.triangles.len());
    for triangle in &mesh.triangles {
        println!("{} {} {}", triangle[0] as i32, triangle[1] as i32, triangle[2] as i32);
    }
}
