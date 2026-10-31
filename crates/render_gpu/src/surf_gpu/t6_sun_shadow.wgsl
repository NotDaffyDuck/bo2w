// bo2mp: Black Ops II's sun shadow casters: depth only, from the sun, one
// partition per pass.

@group(0) @binding(0) var<uniform> shadow_from_world: mat4x4<f32>;

struct CasterIn {
    @location(0) obj_pos: vec3<f32>,
    @location(6) model_col0: vec4<f32>,
    @location(7) model_col1: vec4<f32>,
    @location(8) model_col2: vec4<f32>,
    @location(9) model_col3: vec4<f32>,
}

@vertex
fn vertex_caster(in: CasterIn) -> @builtin(position) vec4<f32> {
    let model_mat = mat4x4<f32>(
        in.model_col0,
        in.model_col1,
        in.model_col2,
        in.model_col3,
    );
    return shadow_from_world * (model_mat * vec4(in.obj_pos, 1.0));
}
