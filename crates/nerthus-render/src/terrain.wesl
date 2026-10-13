// The terrain's vertex stage: a flat grid lifted by the height map, which
// is sampled between texels at each vertex's coordinates; the rest is
// Bevy's mesh vertex stage, so the standard (unlit) fragment stage draws
// it with the baked colours.

import bevy_pbr::render::{
    mesh_functions,
    forward_io::{Vertex, VertexOutput, decompress_vertex},
    view_transformations::position_world_to_clip,
};

struct TerrainShape {
    // How high a height of 1 stands, the map's size in texels, and whether
    // the grid spans the map twice each way.
    height_scale: f32,
    width: u32,
    height: u32,
    tiled: u32,
}

@group(constants::MATERIAL_BIND_GROUP) @binding(100) var<uniform> shape: TerrainShape;
@group(constants::MATERIAL_BIND_GROUP) @binding(101) var heights: texture_2d<f32>;

// A texel of the height map: clamped to the edges, or wrapping when tiled.
fn texel(x: i32, y: i32) -> f32 {
    let size = vec2<i32>(i32(shape.width), i32(shape.height));
    var coordinates = vec2<i32>(x, y);
    if shape.tiled != 0u {
        coordinates = ((coordinates % size) + size) % size;
    } else {
        coordinates = clamp(coordinates, vec2<i32>(0), size - vec2<i32>(1));
    }
    return textureLoad(heights, coordinates, 0).r;
}

// The height at `uv` (texel centres at half texels), between the four
// texels round it.
fn height_at(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(f32(shape.width), f32(shape.height));
    let position = uv * size - vec2<f32>(0.5);
    let corner = floor(position);
    let fraction = position - corner;
    let x = i32(corner.x);
    let y = i32(corner.y);
    let top = mix(texel(x, y), texel(x + 1, y), fraction.x);
    let bottom = mix(texel(x, y + 1), texel(x + 1, y + 1), fraction.x);
    return mix(top, bottom, fraction.y);
}

@vertex
fn vertex(vertex_in: Vertex) -> VertexOutput {
    var out: VertexOutput;
    let vertex = decompress_vertex(vertex_in, vertex_in.instance_index);
    let world_from_local = mesh_functions::get_world_from_local(vertex_in.instance_index);

    var position = vertex.position;
    position.y = height_at(vertex.uv) * shape.height_scale;
    out.world_position = mesh_functions::mesh_position_local_to_world(world_from_local, vec4<f32>(position, 1.0));
    out.position = position_world_to_clip(out.world_position.xyz);
    // Unlit: the normal is never read.
    out.world_normal = vec3<f32>(0.0, 1.0, 0.0);
    out.uv = vertex.uv;

@if(VERTEX_OUTPUT_INSTANCE_INDEX)
    out.instance_index = vertex_in.instance_index;

@if(VISIBILITY_RANGE_DITHER)
    out.visibility_range_dither = mesh_functions::get_visibility_range_dither_level(
        vertex_in.instance_index, world_from_local[3]);

    return out;
}
