@group(0) @binding(0) var encoded: texture_2d<f32>;
@group(0) @binding(1) var linear: texture_storage_2d<rgba16float, write>;
@compute @workgroup_size(16, 16)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(encoded);
    if id.x >= size.x || id.y >= size.y { return; }
    let pixel = textureLoad(encoded, vec2<i32>(id.xy), 0);
    let rgb = select(pow((pixel.rgb + 0.055) / 1.055, vec3(2.4)), pixel.rgb / 12.92, pixel.rgb <= vec3(0.04045));
    textureStore(linear, vec2<i32>(id.xy), vec4(rgb, pixel.a));
}
