struct Uniforms { split: f32, srgb_target: f32, reserved: vec2<f32> }
@group(0) @binding(0) var original: texture_2d<f32>;
@group(0) @binding(1) var edited: texture_2d<f32>;
@group(0) @binding(2) var sampler_linear: sampler;
@group(0) @binding(3) var<uniform> uniforms: Uniforms;
struct Vertex { @builtin(position) position: vec4<f32>, @location(0) uv: vec2<f32> }
@vertex fn vs_main(@builtin(vertex_index) i: u32) -> Vertex {
    var coords = array<vec2<f32>, 6>(vec2(0., 0.), vec2(1., 0.), vec2(0., 1.), vec2(0., 1.), vec2(1., 0.), vec2(1., 1.));
    let uv = coords[i];
    return Vertex(vec4(uv.x * 2. - 1., 1. - uv.y * 2., 0., 1.), uv);
}
fn encode_srgb(v: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(max(v, vec3(0.)), vec3(1. / 2.4)) - 0.055, v * 12.92, v <= vec3(0.0031308));
}
fn decode_srgb(v: vec3<f32>) -> vec3<f32> {
    return select(pow((v + 0.055) / 1.055, vec3(2.4)), v / 12.92, v <= vec3(0.04045));
}
@fragment fn fs_main(v: Vertex) -> @location(0) vec4<f32> {
    let before = textureSample(original, sampler_linear, v.uv);
    let after = textureSample(edited, sampler_linear, v.uv);
    let pixel = select(after, before, v.uv.x < uniforms.split);
    let checker = 0.12 + f32((u32(v.position.x / 12.) + u32(v.position.y / 12.)) % 2u) * 0.035;
    let display = encode_srgb(pixel.rgb) * pixel.a + vec3(checker) * (1. - pixel.a);
    return vec4(select(display, decode_srgb(display), uniforms.srgb_target > 0.5), 1.);
}
