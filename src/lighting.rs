//! Tonal endpoints and painted dodge/burn.

fn luma(p: &[f32; 4]) -> f32 {
    p[0] * 0.2126 + p[1] * 0.7152 + p[2] * 0.0722
}
pub fn apply(mut pixel: [f32; 4], whites: f32, blacks: f32, local_ev: f32) -> [f32; 4] {
    if whites == 0. && blacks == 0. && local_ev == 0. {
        return pixel;
    }
    let luminance = luma(&pixel).clamp(0., 1.);
    let white_weight = smooth(0.3, 0.95, luminance);
    let black_weight = 1. - smooth(0.01, 0.2, luminance);
    let gain = 2_f32.powf(whites / 100. * white_weight + local_ev);
    for value in &mut pixel[..3] {
        *value = (*value + blacks / 100. * 0.06 * black_weight).max(0.) * gain;
    }
    pixel
}
fn smooth(min: f32, max: f32, value: f32) -> f32 {
    let t = ((value - min) / (max - min)).clamp(0., 1.);
    t * t * (3. - 2. * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tonal_endpoints_and_local_light_have_distinct_scopes_and_keep_alpha() {
        let dark = [0.01, 0.01, 0.01, 0.4];
        let bright = [0.8, 0.8, 0.8, 0.6];
        assert!(apply(dark, 0., 60., 0.)[0] > dark[0]);
        assert!((apply(bright, 0., 60., 0.)[0] - bright[0]).abs() < 0.00001);
        assert!(apply(bright, -60., 0., 0.)[0] < bright[0]);
        let colored = [0.3, 0.2, 0.1, 0.7];
        let lit = apply(colored, 0., 0., 1.);
        assert_eq!(lit, [0.6, 0.4, 0.2, 0.7]);
        assert_eq!(apply(colored, 0., 0., 0.), colored);
    }
}
