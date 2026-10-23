//! The GPU behind agent visuals.
//!
//! GPUI composites images, not foreign GPU passes, so visuals that need the
//! GPU (fragment shaders, lit 3D scenes) render offscreen here through wgpu
//! (Metal, Vulkan, or Direct3D) into a texture that is read back as
//! straight-alpha BGRA and painted like any other image, clipped, faded, and
//! layered with the transcript. One device serves every visual; pipelines are
//! cached by source, so a shader compiles once however many frames it draws.

use std::sync::OnceLock;

mod scene;
mod shader;
mod textures;

pub use scene::{SceneState, render_scene};
pub use shader::{ShaderSpec, ShaderState, render_shader};

/// One rendered frame: straight-alpha BGRA, top row first.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

pub(crate) struct Gpu {
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) shaders: OnceLock<shader::Pipelines>,
    pub(crate) scene: OnceLock<scene::Pipelines>,
    pub(crate) textures: textures::Textures,
}

/// The shared device, made on first use; `None` without a usable adapter.
pub(crate) fn gpu() -> Option<&'static Gpu> {
    device().as_ref().ok()
}

fn device() -> &'static Result<Gpu, String> {
    static GPU: OnceLock<Result<Gpu, String>> = OnceLock::new();
    GPU.get_or_init(|| {
        pollster::block_on(Gpu::new())
            .inspect_err(|error| tracing::warn!(%error, "visual GPU unavailable"))
    })
}

/// Run `f`, catching every error wgpu reports while it runs (validation,
/// internal, out of memory) as this call's error.
pub(crate) fn guarded<T>(device: &wgpu::Device, f: impl FnOnce() -> T) -> Result<T, String> {
    let scopes = [
        wgpu::ErrorFilter::OutOfMemory,
        wgpu::ErrorFilter::Internal,
        wgpu::ErrorFilter::Validation,
    ]
    .map(|filter| device.push_error_scope(filter));
    let out = f();
    let mut error = None;
    for scope in scopes.into_iter().rev() {
        if let Some(e) = pollster::block_on(scope.pop()) {
            error.get_or_insert_with(|| e.to_string());
        }
    }
    match error {
        Some(error) => Err(error),
        None => Ok(out),
    }
}

/// Why visuals can't use the GPU here, if they can't.
pub fn unavailable_reason() -> Option<String> {
    device().as_ref().err().cloned()
}

/// Whether visuals can render on the GPU here.
pub fn available() -> bool {
    gpu().is_some()
}

/// The target every visual renders into before readback.
pub(crate) const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Bound render targets by both our memory ceiling and the adapter's limit.
pub(crate) fn target_dimension(pixels: f32, device_limit: u32) -> u32 {
    pixels.round().max(1.0).min(device_limit.clamp(1, 8192) as f32) as u32
}

/// WGSL for the final passes: how much of a pixel survives near the frame's
/// edges, so light running off a visual fades out instead of stopping at a
/// hard line. The ramp is 15% of the shorter side, eased at both ends
/// (smootherstep) so neither the start nor the end of the fade shows.
pub(crate) const EDGE_FADE: &str = r#"
fn edge_ease(d: f32, width: f32) -> f32 {
    let t = clamp(d / width, 0.0, 1.0);
    return t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
}

fn edge_fade(pixel: vec2<f32>, size: vec2<f32>) -> f32 {
    let width = max(0.15 * min(size.x, size.y), 1.0);
    let d = min(pixel, size - pixel);
    return edge_ease(d.x, width) * edge_ease(d.y, width);
}
"#;

impl Gpu {
    async fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            flags: wgpu::InstanceFlags::default(),
            backend_options: wgpu::BackendOptions::default(),
            memory_budget_thresholds: wgpu::MemoryBudgetThresholds::default(),
            display: None,
        });
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::LowPower,
                force_fallback_adapter: false,
                compatible_surface: None,
            })
            .await
            .map_err(|e| e.to_string())?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("rcode visuals"),
                // What this adapter has, not wgpu's defaults: some devices
                // (the iOS simulator) fall short of a default limit the
                // renderers never come near.
                required_limits: adapter.limits(),
                ..Default::default()
            })
            .await
            .map_err(|e| e.to_string())?;
        // wgpu panics on an error nothing catches; a visual's mistake (or a
        // lost device) must cost that visual, not the app.
        device.on_uncaptured_error(std::sync::Arc::new(|error| {
            tracing::warn!(%error, "visual GPU error");
        }));
        Ok(Self {
            device,
            queue,
            shaders: OnceLock::new(),
            scene: OnceLock::new(),
            textures: Default::default(),
        })
    }

    /// Read a rendered texture back as straight-alpha BGRA, through a
    /// staging buffer kept in `staging` between frames.
    pub(crate) fn read_back(
        &self,
        mut encoder: wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        staging: &mut Option<wgpu::Buffer>,
    ) -> Result<Frame, String> {
        let (width, height) = (texture.width(), texture.height());
        let row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let size = u64::from(row * height);
        if staging.as_ref().is_none_or(|b| b.size() != size) {
            *staging = Some(self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("visual readback"),
                size,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }));
        }
        let buffer = staging.as_ref().expect("made above");
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(row),
                    rows_per_image: None,
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .map_err(|error| format!("GPU readback poll: {error}"))?;
        rx.recv()
            .map_err(|error| format!("GPU readback callback: {error}"))?
            .map_err(|error| format!("GPU readback map: {error}"))?;
        let mut bgra = vec![0u8; (width * height * 4) as usize];
        {
            let mapped = buffer.slice(..).get_mapped_range();
            for y in 0..height as usize {
                let src = &mapped[y * row as usize..][..width as usize * 4];
                let dst = &mut bgra[y * width as usize * 4..][..width as usize * 4];
                for (d, s) in dst.chunks_exact_mut(4).zip(src.chunks_exact(4)) {
                    d[0] = s[2];
                    d[1] = s[1];
                    d[2] = s[0];
                    d[3] = s[3];
                }
            }
            drop(mapped);
            buffer.unmap();
        }
        Ok(Frame {
            width,
            height,
            bgra,
        })
    }

    pub(crate) fn target(&self, width: u32, height: u32, label: &str) -> wgpu::Texture {
        self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        })
    }
}

#[cfg(test)]
mod target_tests {
    #[test]
    fn dimensions_fit_memory_and_adapter_limits() {
        assert_eq!(super::target_dimension(1e20, 16384), 8192);
        assert_eq!(super::target_dimension(8192.0, 2048), 2048);
        assert_eq!(super::target_dimension(0.0, 8192), 1);
        assert_eq!(super::target_dimension(f32::NAN, 8192), 1);
        assert_eq!(super::target_dimension(f32::INFINITY, 8192), 8192);
    }
}

#[cfg(test)]
pub(crate) fn dump_ppm(frame: &Frame, path: &str) {
    let mut out = format!("P6\n{} {}\n255\n", frame.width, frame.height).into_bytes();
    for p in frame.bgra.chunks_exact(4) {
        // Over a mid grey, so transparency reads.
        let a = f32::from(p[3]) / 255.0;
        for ch in [p[2], p[1], p[0]] {
            out.push((f32::from(ch) * a + 60.0 * (1.0 - a)) as u8);
        }
    }
    let _ = std::fs::write(path, out);
}
