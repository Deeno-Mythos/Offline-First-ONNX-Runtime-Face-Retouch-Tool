//! egui-wgpu photo callback with compute-converted linear half-float textures.
use crate::engine::srgb_to_linear;
use eframe::egui;
use egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor, wgpu};
use image::RgbaImage;
use std::sync::Arc;

pub struct Canvas {
    pub key: (u64, u64),
    pub original: Arc<RgbaImage>,
    pub edited: Arc<RgbaImage>,
    pub split: f32,
    pub show_original: bool,
}

struct Resources {
    format: wgpu::TextureFormat,
    render: wgpu::RenderPipeline,
    convert: wgpu::ComputePipeline,
    texture_layout: wgpu::BindGroupLayout,
    convert_layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    sampler: wgpu::Sampler,
    textures: Option<(wgpu::Texture, wgpu::Texture, wgpu::BindGroup)>,
    key: Option<(u64, u64)>,
}

pub fn install(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(state) = &cc.wgpu_render_state else {
        return false;
    };
    let device = &state.device;
    let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Astra linear canvas layout"),
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
        label: Some("Astra photo canvas"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/canvas.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&texture_layout],
        push_constant_ranges: &[],
    });
    let render = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Astra canvas pipeline"),
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
    let convert_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Astra linear working texture layout"),
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
        label: Some("Astra linear-light compute"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/linear.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[&convert_layout],
        push_constant_ranges: &[],
    });
    let convert = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
        label: Some("Astra sRGB to linear compute"),
        layout: Some(&layout),
        module: &shader,
        entry_point: Some("main"),
        compilation_options: Default::default(),
        cache: None,
    });
    let uniform = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Astra canvas uniforms"),
        size: 16,
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
        convert,
        texture_layout,
        convert_layout,
        uniform,
        sampler,
        textures: None,
        key: None,
    });
    true
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    encoder: &mut wgpu::CommandEncoder,
    resources: &Resources,
    image: &RgbaImage,
) -> wgpu::Texture {
    let size = wgpu::Extent3d {
        width: image.width(),
        height: image.height(),
        depth_or_array_layers: 1,
    };
    let encoded = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Astra encoded input"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &encoded,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        image.as_raw(),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(image.width() * 4),
            rows_per_image: Some(image.height()),
        },
        size,
    );
    let linear = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Astra Rgba16Float working image"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::STORAGE_BINDING,
        view_formats: &[],
    });
    let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &resources.convert_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(
                    &encoded.create_view(&Default::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &linear.create_view(&Default::default()),
                ),
            },
        ],
    });
    {
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("Astra linearize image"),
            timestamp_writes: None,
        });
        pass.set_pipeline(&resources.convert);
        pass.set_bind_group(0, &bind, &[]);
        pass.dispatch_workgroups(image.width().div_ceil(16), image.height().div_ceil(16), 1);
    }
    linear
}

impl CallbackTrait for Canvas {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(resources) = callback_resources.get_mut::<Resources>() else {
            return vec![];
        };
        if resources.key != Some(self.key) {
            let original = upload(device, queue, encoder, resources, &self.original);
            let edited = upload(device, queue, encoder, resources, &self.edited);
            let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Astra before/after image pair"),
                layout: &resources.texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(
                            &original.create_view(&Default::default()),
                        ),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(
                            &edited.create_view(&Default::default()),
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
            });
            resources.textures = Some((original, edited, bind));
            resources.key = Some(self.key);
        }
        let uniform = [
            if self.show_original { 1.1 } else { self.split },
            if resources.format.is_srgb() { 1.0 } else { 0.0 },
            0.0,
            0.0,
        ];
        queue.write_buffer(&resources.uniform, 0, bytemuck::cast_slice(&uniform));
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
        if let Some((_, _, bind)) = &resources.textures {
            pass.set_pipeline(&resources.render);
            pass.set_bind_group(0, bind, &[]);
            pass.draw(0..6, 0..1);
        }
    }
}

pub fn linear_half_pixels(image: &RgbaImage) -> Vec<half::f16> {
    image
        .as_raw()
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| {
            [
                half::f16::from_f32(srgb_to_linear(p[0] as f32 / 255.0)),
                half::f16::from_f32(srgb_to_linear(p[1] as f32 / 255.0)),
                half::f16::from_f32(srgb_to_linear(p[2] as f32 / 255.0)),
                half::f16::from_f32(p[3] as f32 / 255.0),
            ]
        })
        .collect()
}
