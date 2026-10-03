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
