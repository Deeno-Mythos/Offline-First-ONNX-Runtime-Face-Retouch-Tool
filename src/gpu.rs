//! egui-wgpu photo callback with compute-converted linear half-float textures.
use crate::engine::linear_byte;
use eframe::egui;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use image::RgbaImage;
use std::sync::Arc;

#[doc(hidden)]
#[path = "gpu_diagnostics.rs"]
pub mod diagnostics;

pub struct Canvas {
    pub key: (u64, u64),
    pub image_rect: egui::Rect,
    pub paint_rect: egui::Rect,
    pub original: Arc<RgbaImage>,
    pub edited: Arc<RgbaImage>,
    pub split: f32,
    pub show_original: bool,
}

struct Resources {
    format: wgpu::TextureFormat,
    render: wgpu::RenderPipeline,
    upload: LinearUpload,
    texture_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    original: Option<WorkingImage>,
    edited: Option<WorkingImage>,
    bind: Option<wgpu::BindGroup>,
    uniform_value: Option<[f32; 8]>,
}

pub fn install(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(state) = &cc.wgpu_render_state else {
        return false;
    };
    let device = &state.device;
    let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Hastur linear canvas layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Hastur photo canvas"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/canvas.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&texture_layout],
        push_constant_ranges: &[],
    });
    let render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Hastur canvas pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: state.target_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: Default::default(),
        depth_stencil: None,
        multisample: Default::default(),
        multiview: None,
        cache: None,
    });
    let upload = LinearUpload::new(device);
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Hastur canvas uniforms"),
        size: 32,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: None,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });
    state.renderer.write().callback_resources.insert(Resources {
        format: state.target_format,
        render,
        upload,
        texture_layout,
        uniform,
        sampler,
        original: None,
        edited: None,
        bind: None,
        uniform_value: None,
    });
    true
}

struct LinearUpload {
    convert: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
}
impl LinearUpload {
    fn new(device: &wgpu::Device) -> Self {
        let convert_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hastur linear working texture layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D2,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Hastur linear-light compute"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/linear.wgsl").into()),
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[&convert_layout],
            push_constant_ranges: &[],
        });
        let convert = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Hastur sRGB to linear compute"),
            layout: Some(&layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self {
            convert,
            layout: convert_layout,
        }
    }
}

struct WorkingImage {
    encoded: wgpu::Texture,
    linear: wgpu::Texture,
    convert_bind: wgpu::BindGroup,
    convert_region: wgpu::Buffer,
    image: Arc<RgbaImage>,
}
pub fn release_images(state: &egui_wgpu::RenderState) {
    if let Some(resources) = state
        .renderer
        .write()
        .callback_resources
        .get_mut::<Resources>()
    {
        resources.bind = None;
        resources.original = None;
        resources.edited = None;
        resources.uniform_value = None;
    }
}
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    resources: &LinearUpload,
    image: &Arc<RgbaImage>,
    previous: Option<WorkingImage>,
    incremental: bool,
) -> (WorkingImage, bool, u64) {
    if let Some(old) = &previous
        && Arc::ptr_eq(&old.image, image)
    {
        return (previous.unwrap(), false, 0);
    }
    let allocate = previous
        .as_ref()
        .is_none_or(|old| old.image.dimensions() != image.dimensions());
    let size = wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let region = if incremental && !allocate {
        crate::pixel_changes::bounds(
            &previous.as_ref().unwrap().image,
            image,
            crate::pixel_changes::Scan::Auto,
        )
    } else {
        Some(crate::engine::Crop {
            x: 0,
            y: 0,
            width: image.width(),
            height: image.height(),
        })
    };
    let Some(region) = region else {
        let mut old = previous.unwrap();
        old.image = image.clone();
        return (old, false, 0);
    };
    let working = if allocate {
        let encoded = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hastur encoded input"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let linear = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hastur Rgba16Float working image"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::STORAGE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let convert_region = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Hastur changed pixel region"),
            size: 16,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let convert_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &resources.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &encoded.create_view(&Default::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: convert_region.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(
                        &linear.create_view(&Default::default()),
                    ),
                },
            ],
        });
        WorkingImage {
            encoded,
            linear,
            convert_bind,
            convert_region,
            image: image.clone(),
        }
    } else {
        let mut old = previous.unwrap();
        old.image = image.clone();
        old
    };
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &working.encoded,
            mip_level: 0,
            origin: wgpu::Origin3d {
                x: region.x,
                y: region.y,
                z: 0,
            },
            aspect: wgpu::TextureAspect::All,
        },
        &image.as_raw()[((region.y as usize * image.width() as usize + region.x as usize) * 4)..],
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width() * 4),
            rows_per_image: Some(image.height()),
        },
        wgpu::Extent3d {
            width: region.width,
            height: region.height,
            depth_or_array_layers: 1,
        },
    );
    queue.write_buffer(
        &working.convert_region,
        0,
        bytemuck::cast_slice(&[region.x, region.y, region.width, region.height]),
    );
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Hastur linearize image"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&resources.convert);
        pass.set_bind_group(0, &working.convert_bind, &[]);
        pass.dispatch_workgroups(region.width.div_ceil(16), region.height.div_ceil(16), 1);
    }
    (
        working,
        allocate,
        region.width as u64 * region.height as u64,
    )
}

impl CallbackTrait for Canvas {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = callback_resources.get_mut::<Resources>() else {
            return vec![];
        };
        let old_original = resources.original.take();
        let old_edited = resources.edited.take();
        let (original, changed_original, _) = upload(
            device,
            queue,
            encoder,
            &resources.upload,
            &self.original,
            old_original,
            true,
        );
        let (edited, changed_edited, _) = upload(
            device,
            queue,
            encoder,
            &resources.upload,
            &self.edited,
            old_edited,
            true,
        );
        if changed_original || changed_edited || resources.bind.is_none() {
            resources.bind = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Hastur before/after image pair"),
                layout: &resources.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &original.linear.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &edited.linear.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(&resources.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: resources.uniform.as_entire_binding(),
                    },
                ],
            }));
        }
        resources.original = Some(original);
        resources.edited = Some(edited);
        let uv = canvas_uv(self.image_rect, self.paint_rect, screen);
        let uniform = [
            if self.show_original { 1.1 } else { self.split },
            if resources.format.is_srgb() { 1.0 } else { 0.0 },
            0.0,
            0.0,
            uv[0],
            uv[1],
            uv[2],
            uv[3],
        ];
        if resources.uniform_value != Some(uniform) {
            queue.write_buffer(&resources.uniform, 0, bytemuck::cast_slice(&uniform));
            resources.uniform_value = Some(uniform);
        }
        vec![]
    }
    fn paint(
        &self,
        _: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        let Some(resources) = callback_resources.get::<Resources>() else {
            return;
        };
        if let Some(bind) = &resources.bind {
            pass.set_pipeline(&resources.render);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

/// Match the backend's rounded physical viewport, sampling only its part of the image.
/// Offscreen image bounds must never become a clamped viewport with full-range UVs.
fn canvas_uv(image: egui::Rect, paint: egui::Rect, screen: &ScreenDescriptor) -> [f32; 4] {
    let viewport = egui::PaintCallbackInfo {
        viewport: paint,
        clip_rect: paint,
        pixels_per_point: screen.pixels_per_point,
        screen_size_px: screen.size_in_pixels,
    }
    .viewport_in_pixels();
    let dpi = screen.pixels_per_point;
    let origin = egui::pos2(viewport.left_px as f32, viewport.top_px as f32) / dpi;
    let size = egui::vec2(viewport.width_px as f32, viewport.height_px as f32) / dpi;
    let uv = (origin - image.min) / image.size();
    let extent = size / image.size();
    [uv.x, uv.y, extent.x, extent.y]
}

pub fn linear_half_pixels(image: &RgbaImage) -> Vec<half::f16> {
    image
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            [
                half::f16::from_f32(linear_byte(p[0])),
                half::f16::from_f32(linear_byte(p[1])),
                half::f16::from_f32(linear_byte(p[2])),
                half::f16::from_f32(p[3] as f32 / 255.0),
            ]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipped_zoom_keeps_equal_source_pixel_scale_on_both_axes_and_at_each_dpi() {
        for dpi in [1.0, 1.25, 2.0] {
            let screen = ScreenDescriptor {
                size_in_pixels: [1800, 1200],
                pixels_per_point: dpi,
            };
            let panel = egui::Rect::from_min_size(egui::pos2(37.3, 81.8), egui::vec2(640.4, 440.3));
            for dimensions in [egui::vec2(3000., 2000.), egui::vec2(4000., 6000.)] {
                for scale in [0.5, 1.0, 2.0, 8.0] {
                    let image = egui::Rect::from_center_size(
                        panel.center() + egui::vec2(63.0, -41.0),
                        dimensions * scale / dpi,
                    );
                    let paint = image.intersect(panel);
                    let uv = canvas_uv(image, paint, &screen);
                    let physical = egui::PaintCallbackInfo {
                        viewport: paint,
                        clip_rect: paint,
                        pixels_per_point: dpi,
                        screen_size_px: screen.size_in_pixels,
                    }
                    .viewport_in_pixels();
                    let source_per_pixel = [
                        uv[2] * dimensions.x / physical.width_px as f32,
                        uv[3] * dimensions.y / physical.height_px as f32,
                    ];
                    for value in source_per_pixel {
                        assert!((value - 1.0 / scale).abs() < 0.000001);
                    }
                    let first_source = uv[0] * dimensions.x + source_per_pixel[0] * 0.5;
                    let expected = (physical.left_px as f32 + 0.5 - image.left() * dpi) / scale;
                    assert!((first_source - expected).abs() < 0.001);
                }
            }
        }
    }
}
