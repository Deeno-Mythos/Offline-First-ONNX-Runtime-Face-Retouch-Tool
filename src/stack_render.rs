//! Layer compositing at the requested source resolution. No overview bitmap is saved as a layer.
use crate::{
    engine::{self, Crop, Edit, Renderer, Segmentation},
    layer_stack::{BlendMode, Layer, LayerType},
};
use anyhow::Result;
use image::RgbaImage;
use rayon::prelude::*;
use std::sync::{
    Arc, Weak,
    atomic::{AtomicBool, Ordering},
};

struct Prefix {
    source: Weak<RgbaImage>,
    seg: Option<Weak<Segmentation>>,
    layers: Vec<Layer>,
    image: Arc<RgbaImage>,
}
#[derive(Default)]
pub(crate) struct StackRenderer {
    flat: Option<Box<Renderer>>,
    prefixes: Vec<Prefix>,
}
fn same_layers(a: &[Layer], b: &[Layer]) -> bool {
    a.len() == b.len()
        && a.iter().zip(b).all(|(a, b)| {
            a.kind == b.kind
                && a.opacity == b.opacity
                && a.blend == b.blend
                && a.mask == b.mask
                && a.edit == b.edit
        })
}
fn same_seg(a: &Option<Weak<Segmentation>>, b: Option<&Arc<Segmentation>>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => a.ptr_eq(&Arc::downgrade(b)),
        _ => false,
    }
}
fn cancelled(cancel: Option<&AtomicBool>) -> Result<()> {
    anyhow::ensure!(
        !cancel.is_some_and(|c| c.load(Ordering::Relaxed)),
        "Layer rendering cancelled"
    );
    Ok(())
}
impl StackRenderer {
    fn flat(&mut self) -> &mut Renderer {
        self.flat
            .get_or_insert_with(|| Box::new(Renderer::default()))
    }
    fn prefix(
        &mut self,
        source: &Arc<RgbaImage>,
        layers: &[Layer],
        seg: Option<&Arc<Segmentation>>,
        cancel: Option<&AtomicBool>,
    ) -> Result<Arc<RgbaImage>> {
        cancelled(cancel)?;
        if layers.is_empty() {
            return Ok(source.clone());
        }
        let hit = self
            .prefixes
            .iter()
            .enumerate()
            .filter(|(_, p)| {
                p.source.ptr_eq(&Arc::downgrade(source))
                    && same_seg(&p.seg, seg)
                    && p.layers.len() <= layers.len()
                    && same_layers(&p.layers, &layers[..p.layers.len()])
            })
            .max_by_key(|(_, p)| p.layers.len())
            .map(|(i, _)| i);
        let (mut image, start) = if let Some(index) = hit {
            let prefix = self.prefixes.remove(index);
            let pair = (prefix.image.clone(), prefix.layers.len());
            self.prefixes.push(prefix);
            pair
        } else {
            (source.clone(), 0)
        };
        for (index, layer) in layers.iter().enumerate().skip(start) {
            cancelled(cancel)?;
            let rendered = if layer.kind == LayerType::OriginalCopy {
                self.flat().render(source, &layer.edit, seg, cancel)?
            } else {
                self.flat().render(&image, &layer.edit, seg, cancel)?
            };
            let coverage = layer.mask.as_ref().map(|mask| {
                mask.coverage(
                    source.dimensions(),
                    Crop {
                        x: 0,
                        y: 0,
                        width: source.width(),
                        height: source.height(),
                    },
                )
            });
            image = Arc::new(composite(
                &image,
                rendered,
                layer.factor(),
                layer.blend,
                layer.kind == LayerType::Retouch,
                coverage.as_deref(),
            ));
            self.prefixes.push(Prefix {
                source: Arc::downgrade(source),
                seg: seg.map(Arc::downgrade),
                layers: layers[..=index].to_vec(),
                image: image.clone(),
            });
            while self.prefixes.len() > 2
                || self
                    .prefixes
                    .iter()
                    .map(|p| p.image.as_raw().len())
                    .sum::<usize>()
                    > 192 * 1024 * 1024
            {
                self.prefixes.remove(0);
            }
        }
        Ok(image)
    }
    pub(crate) fn render(
        &mut self,
        source: &Arc<RgbaImage>,
        edit: &Edit,
        seg: Option<&Arc<Segmentation>>,
        cancel: Option<&AtomicBool>,
    ) -> Result<RgbaImage> {
        let layers = edit.resolved_layers();
        Ok((*self.prefix(source, &layers, seg, cancel)?).clone())
    }
    pub(crate) fn region(
        &mut self,
        source: &Arc<RgbaImage>,
        edit: &Edit,
        seg: Option<&Arc<Segmentation>>,
        crop: Crop,
        cancel: Option<&AtomicBool>,
    ) -> Result<RgbaImage> {
        let layers = edit.resolved_layers();
        let Some((top, below)) = layers.split_last() else {
            return self
                .flat()
                .render_region(source, &Edit::default(), seg, crop, cancel);
        };
        let prefix = self.prefix(source, below, seg, cancel)?;
        let input = if top.kind == LayerType::OriginalCopy {
            source
        } else {
            &prefix
        };
        let rendered = self
            .flat()
            .render_region(input, &top.edit, seg, crop, cancel)?;
        let base =
            image::imageops::crop_imm(&*prefix, crop.x, crop.y, crop.width, crop.height).to_image();
        let coverage = top
            .mask
            .as_ref()
            .map(|mask| mask.coverage(source.dimensions(), crop));
        Ok(composite(
            &base,
            rendered,
            top.factor(),
            top.blend,
            top.kind == LayerType::Retouch,
            coverage.as_deref(),
        ))
    }
}

fn composite(
    base: &RgbaImage,
    mut top: RgbaImage,
    opacity: f32,
    mode: BlendMode,
    sparse: bool,
    mask: Option<&[u8]>,
) -> RgbaImage {
    if opacity == 1.0 && mode == BlendMode::Normal && mask.is_none() {
        return top;
    }
    top.as_mut()
        .par_chunks_mut(4)
        .zip(base.as_raw().par_chunks(4))
        .enumerate()
        .for_each(|(index, (top, base))| {
            let amount = opacity * mask.map_or(1.0, |mask| mask[index] as f32 / 255.0);
            if amount <= 0.0 {
                top.copy_from_slice(base);
                return;
            }
            if sparse && top == base {
                return;
            }
            if amount == 1.0 && mode == BlendMode::Normal {
                return;
            }
            // An adjustment layer transforms the input, including its alpha, then cross-fades in linear light.
            for c in 0..3 {
                let a = engine::linear_byte(base[c]);
                let b = engine::linear_byte(top[c]);
                let blended = match mode {
                    BlendMode::Normal => b,
                    BlendMode::Multiply => a * b,
                    BlendMode::Screen => 1.0 - (1.0 - a) * (1.0 - b),
                    BlendMode::Difference => (a - b).abs(),
                };
                top[c] = engine::linear_output_byte(a + (blended - a) * amount);
            }
            top[3] = (base[3] as f32 + (top[3] as f32 - base[3] as f32) * amount)
                .round()
                .clamp(0.0, 255.0) as u8;
        });
    top
}
