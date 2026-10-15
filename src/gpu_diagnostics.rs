//! Headless measurements using the application's actual upload and conversion path.
use super::*;
use anyhow::{Context, Result, ensure};
use std::{
    future::Future,
    task::{Poll, Wake, Waker},
    time::{Duration, Instant},
};

pub fn simd_available() -> bool {
    crate::pixel_changes::accelerated()
}
pub fn change_bounds(
    before: &RgbaImage,
    after: &RgbaImage,
    simd: bool,
) -> Option<crate::engine::Crop> {
    crate::pixel_changes::bounds(
        before,
        after,
        if simd {
            crate::pixel_changes::Scan::Auto
        } else {
            crate::pixel_changes::Scan::Scalar
        },
    )
}

struct ThreadWake(std::thread::Thread);
impl Wake for ThreadWake {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}
fn block_on<F: Future>(future: F) -> Result<F::Output> {
    let waker = Waker::from(Arc::new(ThreadWake(std::thread::current())));
    let mut ctx = std::task::Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Poll::Ready(result) = future.as_mut().poll(&mut ctx) {
            return Ok(result);
        }
        ensure!(Instant::now() < deadline, "GPU initialization timed out");
        std::thread::park_timeout(Duration::from_millis(10));
    }
}
pub struct UploadSample {
    pub encode_ms: f64,
    /// Encode, submit and wait for actual completion; no display refresh included.
    pub complete_ms: f64,
    pub pixels: u64,
}

/// Exercises the same egui tessellation, canvas callback, GPU submission and
/// texture path as the app. Completion excludes physical display refresh.
pub struct FrameProfiler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: egui_wgpu::Renderer,
    target: wgpu::Texture,
    size: [u32; 2],
    pub adapter: String,
}
impl FrameProfiler {
    pub fn new(size: [u32; 2]) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))??;
        let info = adapter.get_info();
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Hastur input feedback profiler"),
            ..Default::default()
        }))??;
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let mut renderer =
            egui_wgpu::Renderer::new(&device, format, egui_wgpu::RendererOptions::PREDICTABLE);
        renderer
            .callback_resources
            .insert(canvas_resources(&device, format));
        let target = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hastur feedback framebuffer"),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Ok(Self {
            device,
            queue,
            renderer,
            target,
            size,
            adapter: format!("{} / {:?}", info.name, info.backend),
        })
    }
    pub fn frame(&mut self, ctx: &egui::Context, output: egui::FullOutput) -> Result<f64> {
        let start = Instant::now();
        let screen = ScreenDescriptor {
            size_in_pixels: self.size,
            pixels_per_point: output.pixels_per_point,
        };
        let paint = ctx.tessellate(output.shapes, output.pixels_per_point);
        for (id, delta) in &output.textures_delta.set {
            self.renderer
                .update_texture(&self.device, &self.queue, *id, delta);
        }
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let commands =
            self.renderer
                .update_buffers(&self.device, &self.queue, &mut encoder, &paint, &screen);
        let view = self.target.create_view(&Default::default());
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Hastur feedback frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            self.renderer
                .render(&mut pass.forget_lifetime(), &paint, &screen);
        }
        let submission = self
            .queue
            .submit(commands.into_iter().chain([encoder.finish()]));
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(5)),
        })?;
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
        Ok(start.elapsed().as_secs_f64() * 1000.0)
    }
    pub fn canvas_upload_pixels(&self) -> [u64; 2] {
        self.renderer
            .callback_resources
            .get::<Resources>()
            .map_or([0; 2], |r| r.upload_pixels)
    }
    pub fn screenshot(&self) -> Result<RgbaImage> {
        let [w, h] = self.size;
        let stride = (w * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: stride as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(5)),
        })?;
        rx.recv_timeout(Duration::from_secs(5))??;
        let data = buffer.slice(..).get_mapped_range();
        let bytes = data
            .chunks(stride as usize)
            .flat_map(|row| row[..w as usize * 4].iter().copied())
            .collect();
        RgbaImage::from_raw(w, h, bytes).context("Invalid framebuffer")
    }
}
pub struct UploadProfiler {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: LinearUpload,
    previous: Option<WorkingImage>,
    incremental: bool,
    pub adapter: String,
}
impl UploadProfiler {
    pub fn new(incremental: bool) -> Result<Self> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))??;
        let info = adapter.get_info();
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("Hastur upload profiler"),
            ..Default::default()
        }))??;
        let pipeline = LinearUpload::new(&device);
        Ok(Self {
            device,
            queue,
            pipeline,
            previous: None,
            incremental,
            adapter: format!("{} / {:?}", info.name, info.backend),
        })
    }
    pub fn update(&mut self, image: &Arc<RgbaImage>) -> Result<UploadSample> {
        ensure!(
            image.width() > 0 && image.height() > 0,
            "Empty upload image"
        );
        let start = Instant::now();
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let (working, _, pixels) = upload(
            &self.device,
            &self.queue,
            &mut encoder,
            &self.pipeline,
            image,
            self.previous.take(),
            self.incremental,
        );
        let encode_ms = start.elapsed().as_secs_f64() * 1000.;
        let submission = self.queue.submit([encoder.finish()]);
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(5)),
        })?;
        self.previous = Some(working);
        Ok(UploadSample {
            encode_ms,
            complete_ms: start.elapsed().as_secs_f64() * 1000.,
            pixels,
        })
    }
    pub fn read_linear(&self) -> Result<Vec<u8>> {
        let image = self.previous.as_ref().context("No uploaded image")?;
        let (w, h) = image.image.dimensions();
        let stride = (w * 8).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Hastur upload verification"),
            size: stride as u64 * h as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &image.linear,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(stride),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        let submission = self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.device.poll(wgpu::PollType::Wait {
            submission_index: Some(submission),
            timeout: Some(Duration::from_secs(5)),
        })?;
        rx.recv_timeout(Duration::from_secs(2))??;
        let data = buffer.slice(..).get_mapped_range();
        let mut pixels = Vec::with_capacity((w * h * 8) as usize);
        for row in data.chunks_exact(stride as usize) {
            pixels.extend_from_slice(&row[..w as usize * 8]);
        }
        drop(data);
        buffer.unmap();
        Ok(pixels)
    }
}

pub fn validate_upload() -> Result<String> {
    let mut full = UploadProfiler::new(false)?;
    let mut live = UploadProfiler::new(true)?;
    let mut photo = Arc::new(RgbaImage::from_fn(257, 193, |x, y| {
        image::Rgba([(x % 255) as u8, (y % 255) as u8, 87, ((x + y) % 256) as u8])
    }));
    for step in 0..12 {
        let mut next = photo.as_ref().clone();
        match step {
            0 | 5 => {}
            1 => next.get_pixel_mut(0, 0)[3] ^= 99,
            2 => next.get_pixel_mut(256, 192)[0] ^= 27,
            3 => {
                for y in 37..58 {
                    for x in 39..66 {
                        next.get_pixel_mut(x, y)[1] ^= 41;
                    }
                }
            }
            4 => {
                next.get_pixel_mut(3, 80)[2] ^= 71;
                next.get_pixel_mut(192, 64)[0] ^= 17;
            }
            6 => next = RgbaImage::from_fn(99, 51, |x, y| image::Rgba([x as u8, y as u8, 55, 180])),
            7 => {
                for p in next.pixels_mut() {
                    p[0] ^= 31;
                }
            }
            _ => {
                let (w, h) = next.dimensions();
                for (x, y) in [(0, 0), (w - 1, h - 1), (w / 2, h / 2)] {
                    next.get_pixel_mut(x, y)[step % 4] ^= 19;
                }
            }
        }
        photo = Arc::new(next);
        full.update(&photo)?;
        let result = live.update(&photo)?;
        if step == 5 {
            ensure!(result.pixels == 0, "Identical new frame must not upload");
        }
        ensure!(
            full.read_linear()? == live.read_linear()?,
            "GPU partial update differs at step {step}"
        );
    }
    Ok(format!(
        "12 GPU readbacks byte-identical, including alpha, corners, odd row stride, disconnected changes, identical new Arc, global changes and resize; {}",
        live.adapter
    ))
}
