//! Exact changed-pixel bounds for retaining unchanged display texture pixels.
use crate::engine::Crop;
use image::RgbaImage;

#[derive(Clone, Copy, Debug)]
pub enum Scan {
    Scalar,
    Auto,
}

pub fn accelerated() -> bool {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        std::is_x86_feature_detected!("avx2")
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    {
        false
    }
}

fn prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).position(|(a, b)| a != b).unwrap_or(a.len())
}
fn suffix(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .rev()
        .zip(b.iter().rev())
        .position(|(a, b)| a != b)
        .unwrap_or(a.len())
}

#[cfg(target_arch = "x86")]
use std::arch::x86::*;
#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn vector_prefix(a: &[u8], b: &[u8]) -> usize {
    let mut i = 0;
    while i + 32 <= a.len() {
        // SAFETY: both same-length slices contain this entire unaligned load.
        let (x, y) = unsafe {
            (
                _mm256_loadu_si256(a.as_ptr().add(i).cast()),
                _mm256_loadu_si256(b.as_ptr().add(i).cast()),
            )
        };
        let different = !(_mm256_movemask_epi8(_mm256_cmpeq_epi8(x, y)) as u32);
        if different != 0 {
            return i + different.trailing_zeros() as usize;
        }
        i += 32;
    }
    i + prefix(&a[i..], &b[i..])
}
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[target_feature(enable = "avx2")]
unsafe fn vector_suffix(a: &[u8], b: &[u8]) -> usize {
    let mut end = a.len();
    while end >= 32 {
        // SAFETY: both same-length slices contain this entire unaligned load.
        let (x, y) = unsafe {
            (
                _mm256_loadu_si256(a.as_ptr().add(end - 32).cast()),
                _mm256_loadu_si256(b.as_ptr().add(end - 32).cast()),
            )
        };
        let different = !(_mm256_movemask_epi8(_mm256_cmpeq_epi8(x, y)) as u32);
        if different != 0 {
            return a.len() - end + different.leading_zeros() as usize;
        }
        end -= 32;
    }
    a.len() - end + suffix(&a[..end], &b[..end])
}
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detected_prefix(a: &[u8], b: &[u8]) -> usize {
    // SAFETY: chosen only after runtime AVX2 detection; bounds validates equal lengths.
    unsafe { vector_prefix(a, b) }
}
#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
fn detected_suffix(a: &[u8], b: &[u8]) -> usize {
    // SAFETY: chosen only after runtime AVX2 detection; bounds validates equal lengths.
    unsafe { vector_suffix(a, b) }
}

type Compare = fn(&[u8], &[u8]) -> usize;
fn comparators(scan: Scan) -> (Compare, Compare) {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    if matches!(scan, Scan::Auto) && accelerated() {
        return (detected_prefix, detected_suffix);
    }
    #[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
    let _ = scan;
    (prefix, suffix)
}
pub fn bounds(before: &RgbaImage, after: &RgbaImage, scan: Scan) -> Option<Crop> {
    let (w, h) = after.dimensions();
    if w == 0 || h == 0 {
        return None;
    }
    if before.dimensions() != (w, h) {
        return Some(Crop {
            x: 0,
            y: 0,
            width: w,
            height: h,
        });
    }
    let (front, back) = comparators(scan);
    // ImageBuffer may retain extra storage after the last actual pixel. Compare
    // only pixels, with equal slice lengths for the bounded SIMD loads.
    let row = w as usize * 4;
    let bytes = row * h as usize;
    let (a, b) = (&before.as_raw()[..bytes], &after.as_raw()[..bytes]);
    let first = front(a, b);
    if first == a.len() {
        return None;
    }
    let last = a.len() - 1 - back(a, b);
    let (top, bottom) = (first / row, last / row);
    let (mut left, mut right) = (first % row / 4, last % row / 4 + 1);
    for y in top..=bottom {
        if left == 0 && right == w as usize {
            break;
        }
        let (a, b) = (&a[y * row..(y + 1) * row], &b[y * row..(y + 1) * row]);
        let first = front(a, b);
        if first != row {
            left = left.min(first / 4);
            right = right.max((row - 1 - back(a, b)) / 4 + 1);
        }
    }
    Some(Crop {
        x: left as u32,
        y: top as u32,
        width: (right - left) as u32,
        height: (bottom - top + 1) as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn reference(a: &RgbaImage, b: &RgbaImage) -> Option<Crop> {
        let (mut x, mut y, mut r, mut d) = (a.width(), a.height(), 0, 0);
        for (px, py, p) in b.enumerate_pixels() {
            if a.get_pixel(px, py) != p {
                x = x.min(px);
                y = y.min(py);
                r = r.max(px + 1);
                d = d.max(py + 1);
            }
        }
        (x < r && y < d).then(|| Crop {
            x,
            y,
            width: r - x,
            height: d - y,
        })
    }
    #[test]
    fn changed_bounds_match_every_pixel_reference_for_tails_edges_alpha_and_disconnected_edits() {
        for (w, h) in [
            (1, 1),
            (7, 3),
            (8, 9),
            (9, 7),
            (31, 33),
            (65, 23),
            (257, 61),
        ] {
            let a = RgbaImage::from_fn(w, h, |x, y| {
                image::Rgba([(x % 251) as u8, (y % 251) as u8, 120, 255])
            });
            assert_eq!(bounds(&a, &a, Scan::Auto), None);
            for seed in 0..72u32 {
                let mut b = a.clone();
                for k in 0..seed % 11 + 1 {
                    let (x, y) = ((seed * 17 + k * 31) % w, (seed * 29 + k * 41) % h);
                    b.get_pixel_mut(x, y)[((seed + k) % 4) as usize] ^= 37;
                }
                let expected = reference(&a, &b);
                assert_eq!(bounds(&a, &b, Scan::Scalar), expected);
                assert_eq!(bounds(&a, &b, Scan::Auto), expected);
            }
        }
        assert_eq!(
            bounds(&RgbaImage::new(3, 2), &RgbaImage::new(7, 5), Scan::Auto),
            Some(Crop {
                x: 0,
                y: 0,
                width: 7,
                height: 5
            })
        );
        assert_eq!(
            bounds(&RgbaImage::new(3, 2), &RgbaImage::new(0, 0), Scan::Auto),
            None
        );
        let padded = RgbaImage::from_raw(7, 5, vec![0; 7 * 5 * 4 + 79]).unwrap();
        let compact = RgbaImage::new(7, 5);
        assert_eq!(bounds(&padded, &compact, Scan::Auto), None);
        assert_eq!(bounds(&compact, &padded, Scan::Auto), None);
    }
}
