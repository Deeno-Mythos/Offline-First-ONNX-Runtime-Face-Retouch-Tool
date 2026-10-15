//! Exact row copies for viewport pixels; no resampling or color conversion.
use crate::engine::Crop;
use image::RgbaImage;

pub(crate) fn crop(image: &RgbaImage, region: Crop) -> RgbaImage {
    let x = region.x.min(image.width());
    let y = region.y.min(image.height());
    let width = region.width.min(image.width() - x);
    let height = region.height.min(image.height() - y);
    let mut out = RgbaImage::new(width, height);
    copy_into(
        image,
        Crop {
            x,
            y,
            width,
            height,
        },
        &mut out,
        [0, 0],
    );
    out
}

pub(crate) fn copy_into(source: &RgbaImage, region: Crop, target: &mut RgbaImage, at: [u32; 2]) {
    let x = region.x.min(source.width());
    let y = region.y.min(source.height());
    let dx = at[0].min(target.width());
    let dy = at[1].min(target.height());
    let width = region
        .width
        .min(source.width() - x)
        .min(target.width() - dx) as usize
        * 4;
    let height = region
        .height
        .min(source.height() - y)
        .min(target.height() - dy);
    if width == 0 || height == 0 {
        return;
    }
    let src_stride = source.width() as usize * 4;
    let dst_stride = target.width() as usize * 4;
    let dst = target.as_mut();
    for row in 0..height as usize {
        let from = (y as usize + row) * src_stride + x as usize * 4;
        let to = (dy as usize + row) * dst_stride + dx as usize * 4;
        dst[to..to + width].copy_from_slice(&source.as_raw()[from..from + width]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn row_copies_match_image_operations_at_edges_with_alpha_and_extra_storage() {
        let mut image = RgbaImage::from_fn(37, 29, |x, y| {
            image::Rgba([x as u8, y as u8, (x * y) as u8, ((x + y) * 3) as u8])
        });
        let mut storage = image.into_raw();
        storage.extend_from_slice(&[211; 100]);
        image = RgbaImage::from_raw(37, 29, storage).unwrap();
        for x in [0, 1, 19, 36, 37] {
            for y in [0, 2, 28, 29] {
                for (width, height) in [(0, 0), (1, 1), (13, 11), (37, 29), (50, 50)] {
                    let region = Crop {
                        x,
                        y,
                        width,
                        height,
                    };
                    let expected =
                        image::imageops::crop_imm(&image, x, y, width, height).to_image();
                    assert!(crop(&image, region) == expected);
                    let mut actual = RgbaImage::new(21, 17);
                    let mut target = actual.clone();
                    image::imageops::replace(&mut target, &expected, 3, 4);
                    copy_into(&image, region, &mut actual, [3, 4]);
                    assert!(actual == target);
                }
            }
        }
    }
}
