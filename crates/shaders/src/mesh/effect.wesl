// Passthrough vertex shader for MeshEffectExtension.
//
// Reproduces StandardMaterial's local to world to clip transform so there is no
// visible change. The wiring exists so real per-vertex effects can be dropped
// in without touching anything else in a vertex shader.
//
// Attribute locations match Bevy's mesh pipeline for now?(future me figure it
// out gotta commit this wip and refactor while it rains freaking cold af out)
//
//   0 = position (always present)
//   1 = normal   (@if(VERTEX_NORMALS))
//   2 = uv_a     (@if(VERTEX_UVS_A))
import bevy_pbr::{
    mesh_functions,
    mesh_vertex_output::MeshVertexOutput,
    view_transformations::position_world_to_clip,
};

struct Vertex {
    @builtin(instance_index) instance_index: u32,
    @location(0) position: vec3<f32>,
    @if(VERTEX_NORMALS)
    @location(1) normal: vec3<f32>,
    @if(VERTEX_UVS_A)
    @location(2) uv: vec2<f32>,
};

@vertex
fn vertex(in: Vertex) -> MeshVertexOutput {
    var out: MeshVertexOutput;

    let world_from_local = mesh_functions::get_world_from_local(in.instance_index);

    out.world_position = mesh_functions::mesh_position_local_to_world(
        world_from_local,
        vec4<f32>(in.position, 1.0),
    );
    out.position = position_world_to_clip(out.world_position.xyz);

    @if(VERTEX_NORMALS)
    {
        out.world_normal = mesh_functions::mesh_normal_local_to_world(
            in.normal,
            in.instance_index,
        );
    }

    @if(VERTEX_UVS_A)
    {
        out.uv = in.uv;
    }

    out.instance_index = in.instance_index;

    return out;
}
