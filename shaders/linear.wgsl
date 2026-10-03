@group(0) @binding(0) var encoded: texture_2d<f32>;
@group(0) @binding(1) var linear: texture_storage_2d<rgba16float, write>;
@group(0) @binding(2) var<uniform> region: vec4<u32>;
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= region.z || id.y >= region.w { return; }
    let coord = region.xy + id.xy;
    let pixel = textureLoad(encoded, vec2<i32>(coord), 0);
    let rgb = select(pow((pixel.rgb + 0.055) / 1.055, vec3(2.4)), pixel.rgb / 12.92, pixel.rgb <= vec3(0.04045));
    textureStore(linear, vec2<i32>(coord), vec4(rgb, pixel.a));
}
