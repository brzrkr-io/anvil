//! Metal renderer.
//!
//! Two coexistent rendering paths:
//!
//! - **CPU path** (`present`): rasterizes the terminal on the CPU into a BGRA8
//!   bitmap and uploads it as a full-screen quad.  This is the active path.
//! - **GPU path** (`present_cells`): draws from a `CellBatch` using per-cell
//!   instance data and a glyph atlas texture.  Phase B infrastructure — not yet
//!   called from `App::render_frame`.
//!
//! Uses `objc2-metal` 0.3 typed bindings.
//! Unsafe blocks are marked with a SAFETY comment explaining the invariant.

use std::ptr::NonNull;
use std::sync::{Arc, Condvar, Mutex};

use anvil_render::CellBatch;
use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLBlendFactor, MTLBlendOperation, MTLBuffer, MTLClearColor, MTLCommandBuffer,
    MTLCommandEncoder, MTLCommandQueue, MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable,
    MTLFunction, MTLLibrary, MTLLoadAction, MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion,
    MTLRenderCommandEncoder, MTLRenderPassDescriptor, MTLRenderPipelineDescriptor,
    MTLRenderPipelineState, MTLResourceOptions, MTLSize, MTLStoreAction, MTLTexture,
    MTLTextureDescriptor,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use thiserror::Error;

const PIXEL_FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm;

/// Initial per-instance-buffer capacity: room for ~8 000 cells.
const INSTANCE_BUF_INIT_BYTES: usize = 8192 * 36; // CellInstance is 36 bytes

/// Number of in-flight frames (triple buffering).
const IN_FLIGHT: usize = 3;

/// Uniforms block uploaded each frame for the cell pipeline: 16 bytes.
#[repr(C)]
struct Uniforms {
    viewport_px: [f32; 2],
    atlas_px: [f32; 2],
}

const SHADER_SRC: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct VOut { float4 pos [[position]]; float2 uv; };
vertex VOut v_main(uint vid [[vertex_id]]) {
    float2 pos[6] = {
        float2(-1,-1), float2(1,-1), float2(-1,1),
        float2(1,-1),  float2(1,1),  float2(-1,1)
    };
    float2 uvs[6] = {
        float2(0,1), float2(1,1), float2(0,0),
        float2(1,1), float2(1,0), float2(0,0)
    };
    VOut o;
    o.pos = float4(pos[vid], 0, 1);
    o.uv = uvs[vid];
    return o;
}
fragment float4 f_main(VOut in [[stage_in]],
                       texture2d<float> tex [[texture(0)]]) {
    constexpr sampler smp(mag_filter::nearest, min_filter::nearest);
    return tex.sample(smp, in.uv);
}
"#;

/// MSL source for the GPU cell pipeline (Phase B).
///
/// Vertex shader: two hardcoded triangles per cell (6 vertices), driven by
/// per-instance `CellInstance` data from buffer(2).  Fragment shader: samples
/// the R8Unorm glyph atlas and blends fg/bg by the alpha channel.
const CELL_SHADER_SRC: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Uniforms {
    float2 viewport_px;
    float2 atlas_px;
};

struct CellInstance {
    packed_float2 cell_px_xy;
    packed_float2 cell_px_wh;
    packed_ushort2 atlas_uv_xy;
    packed_ushort2 atlas_uv_wh;
    packed_short2 glyph_offset;
    uchar4 fg_rgba;
    uchar4 bg_rgba;
};

struct VOut {
    float4 pos [[position]];
    float2 atlas_uv;
    float4 fg;
    float4 bg;
    float glyph_present;
};

vertex VOut cell_v(uint vid [[vertex_id]],
                   uint iid [[instance_id]],
                   constant Uniforms& u [[buffer(1)]],
                   constant CellInstance* cells [[buffer(2)]]) {
    const float2 corners[6] = { {0,0},{1,0},{0,1},{1,0},{1,1},{0,1} };
    float2 c = corners[vid];
    CellInstance ci = cells[iid];

    float2 px = float2(ci.cell_px_xy) + c * float2(ci.cell_px_wh);
    float2 ndc = (px / u.viewport_px) * 2.0 - 1.0;
    ndc.y = -ndc.y;

    VOut o;
    o.pos = float4(ndc, 0, 1);
    o.atlas_uv = float2(ci.atlas_uv_xy) + c * float2(ci.atlas_uv_wh);
    o.fg = float4(ci.fg_rgba) / 255.0;
    o.bg = float4(ci.bg_rgba) / 255.0;
    o.glyph_present = (ci.atlas_uv_wh.x > 0) ? 1.0 : 0.0;
    return o;
}

fragment float4 cell_f(VOut in [[stage_in]],
                       constant Uniforms& u [[buffer(1)]],
                       texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler smp(mag_filter::nearest, min_filter::nearest,
                          coord::pixel, address::clamp_to_edge);
    float a = in.glyph_present * atlas.sample(smp, in.atlas_uv).r;
    return mix(in.bg, in.fg, a);
}
"#;

/// Errors that can occur during renderer setup.
#[derive(Debug, Error)]
pub enum RendererError {
    #[error("no Metal device available")]
    NoMetalDevice,
    #[error("command queue creation failed")]
    CommandQueueFailed,
    #[error("shader compilation failed: {0}")]
    ShaderCompileFailed(String),
    #[error("pipeline creation failed: {0}")]
    PipelineCreationFailed(String),
    #[error("texture creation failed")]
    TextureFailed,
    #[error("buffer creation failed")]
    BufferFailed,
}

/// How to present the current frame.
///
/// `Sync` prevents ghosting during live resize (commit → waitUntilScheduled →
/// present on the main thread, in lockstep with the layer's new drawable size).
/// `Async` gives lower latency for normal frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PresentMode {
    Sync,
    Async,
}

/// Choose the present mode for a frame.  During a live resize the layer must
/// commit synchronously so the frame lands in lockstep with the new drawable
/// size — prevents ghosting.  All other frames use the async path.
pub fn present_mode(in_live_resize: bool) -> PresentMode {
    if in_live_resize {
        PresentMode::Sync
    } else {
        PresentMode::Async
    }
}

/// GPU renderer: owns the Metal device, command queue, pipeline, and texture.
///
/// Two pipelines coexist:
/// - `pipeline` — CPU-upload full-screen blit (the active rendering path).
/// - `cell_pipeline` — per-cell instanced draw (Phase B; not yet called from
///   `App::render_frame`).
pub struct Renderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    layer: Retained<CAMetalLayer>,
    // CPU path (active).
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    width: usize,
    height: usize,
    clear: MTLClearColor,
    // GPU cell path (Phase B — not yet wired into App::render_frame).
    cell_pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    uniforms_buffer: Retained<ProtocolObject<dyn MTLBuffer>>,
    instance_buffers: [Retained<ProtocolObject<dyn MTLBuffer>>; IN_FLIGHT],
    instance_buffer_idx: usize,
    /// Counting semaphore (initialised to IN_FLIGHT = 3) that ensures at most
    /// IN_FLIGHT command buffers are in flight simultaneously.  Uses a plain
    /// `Arc<(Mutex<i32>, Condvar)>` because no `dispatch2` crate is available.
    instance_semaphore: Arc<(Mutex<i32>, Condvar)>,
}

impl Renderer {
    /// Initialise the renderer against an existing `CAMetalLayer`.
    pub fn init(
        layer: Retained<CAMetalLayer>,
        width: usize,
        height: usize,
    ) -> Result<Self, RendererError> {
        // MTLCreateSystemDefaultDevice is a safe extern fn in objc2-metal 0.3.
        let device = MTLCreateSystemDefaultDevice().ok_or(RendererError::NoMetalDevice)?;

        let queue = device
            .newCommandQueue()
            .ok_or(RendererError::CommandQueueFailed)?;

        // Configure the CAMetalLayer.  All setters are safe in objc2-quartz-core 0.3.
        layer.setDevice(Some(&device));
        layer.setPixelFormat(PIXEL_FORMAT);
        // presentsWithTransaction starts false; toggled per-frame in present().
        layer.setPresentsWithTransaction(false);
        layer.setDrawableSize(CGSize {
            width: width as f64,
            height: height as f64,
        });

        let pipeline = build_pipeline(&device)?;
        let texture = make_texture(&device, width, height)?;

        let cell_pipeline = build_cell_pipeline(&device)?;
        let uniforms_buffer = make_buffer(&device, std::mem::size_of::<Uniforms>())
            .ok_or(RendererError::BufferFailed)?;
        let instance_buffers = [
            make_buffer(&device, INSTANCE_BUF_INIT_BYTES).ok_or(RendererError::BufferFailed)?,
            make_buffer(&device, INSTANCE_BUF_INIT_BYTES).ok_or(RendererError::BufferFailed)?,
            make_buffer(&device, INSTANCE_BUF_INIT_BYTES).ok_or(RendererError::BufferFailed)?,
        ];

        // mineral-dark bg; overridden per theme via set_clear_color().
        let clear = rgb_clear_color(0x1a, 0x1c, 0x24);

        Ok(Self {
            device,
            queue,
            layer,
            pipeline,
            texture,
            width,
            height,
            clear,
            cell_pipeline,
            uniforms_buffer,
            instance_buffers,
            instance_buffer_idx: 0,
            instance_semaphore: Arc::new((Mutex::new(IN_FLIGHT as i32), Condvar::new())),
        })
    }

    /// Update the GPU clear colour.  It sits behind the full-screen texture so
    /// it only matters on resize flashes — but it must track the theme.
    pub fn set_clear_color(&mut self, rgb: [u8; 3]) {
        self.clear = rgb_clear_color(rgb[0], rgb[1], rgb[2]);
    }

    /// Recreate the texture (and update the layer drawable size) for a new
    /// viewport.  No-op when dimensions are unchanged.
    pub fn resize(&mut self, width: usize, height: usize) {
        if width == self.width && height == self.height {
            return;
        }
        self.width = width;
        self.height = height;

        self.layer.setDrawableSize(CGSize {
            width: width as f64,
            height: height as f64,
        });

        // Best-effort: if texture creation fails we keep the old texture.
        if let Ok(tex) = make_texture(&self.device, width, height) {
            self.texture = tex;
        }
    }

    /// Upload a `width * height * 4` BGRA8 bitmap and present it as the frame.
    ///
    /// Pass `sync = true` during a live resize to prevent ghosting; for all
    /// other frames `sync = false` uses async `presentDrawable:` for lower
    /// latency.  This exactly mirrors the Zig present path.
    pub fn present(&self, pixels: &[u8], sync: bool) {
        objc2::rc::autoreleasepool(|_| {
            // Toggle the layer property so the command-buffer path matches.
            self.layer.setPresentsWithTransaction(sync);

            // Upload pixels to the texture.
            let region = MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: self.width,
                    height: self.height,
                    depth: 1,
                },
            };
            // SAFETY: pixels is a non-empty slice; the pointer is non-null.
            let bytes_ptr = NonNull::new(pixels.as_ptr() as *mut std::ffi::c_void).unwrap();
            unsafe {
                self.texture
                    .replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        0,
                        bytes_ptr,
                        self.width * 4,
                    );
            }

            // Acquire the next drawable.
            let drawable = match self.layer.nextDrawable() {
                Some(d) => d,
                None => return,
            };
            let dst_texture = drawable.texture();

            // Build render pass descriptor.
            let rpd = MTLRenderPassDescriptor::renderPassDescriptor();
            let color_attachments = rpd.colorAttachments();
            // SAFETY: index 0 always exists on a freshly created descriptor.
            let att = unsafe { color_attachments.objectAtIndexedSubscript(0) };
            att.setTexture(Some(&dst_texture));
            att.setLoadAction(MTLLoadAction::Clear);
            att.setStoreAction(MTLStoreAction::Store);
            att.setClearColor(self.clear);

            // Record the draw call.
            let cmd = match self.queue.commandBuffer() {
                Some(c) => c,
                None => return,
            };
            let enc = match cmd.renderCommandEncoderWithDescriptor(&rpd) {
                Some(e) => e,
                None => return,
            };
            enc.setRenderPipelineState(&self.pipeline);
            // SAFETY: fragment texture at index 0; texture is valid.
            unsafe {
                enc.setFragmentTexture_atIndex(Some(&self.texture), 0);
            }
            // SAFETY: 6 vertices, start 0, triangle primitive.
            unsafe {
                enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
            }
            enc.endEncoding();

            if sync {
                // Synchronous path: commit, wait until scheduled, then
                // present on the main thread so the frame lands in lockstep
                // with the layer's resize — prevents ghosting.
                cmd.commit();
                cmd.waitUntilScheduled();
                // SAFETY: CAMetalDrawable: MTLDrawable — the raw pointer for
                // ProtocolObject<dyn CAMetalDrawable> is valid as
                // &ProtocolObject<dyn MTLDrawable> because the latter is a
                // strict superset of the former's vtable.
                let draw_ref: &ProtocolObject<dyn MTLDrawable> = unsafe {
                    &*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)
                };
                draw_ref.present();
            } else {
                // Async path: lower latency for normal (non-resize) frames.
                // SAFETY: same protocol coercion as the sync path above.
                let draw_ref: &ProtocolObject<dyn MTLDrawable> = unsafe {
                    &*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)
                };
                cmd.presentDrawable(draw_ref);
                cmd.commit();
            }
        });
    }

    /// Draw a `CellBatch` using the GPU cell pipeline.
    ///
    /// Phase B — not yet called from `App::render_frame`; both CPU and GPU
    /// pipelines coexist.
    ///
    /// `atlas` must be an R8Unorm texture produced by `AtlasPainter`.
    /// `atlas_px` is the atlas texture dimensions in pixels.
    /// Pass `sync = true` during live resize (mirrors the `present` path).
    pub fn present_cells(
        &mut self,
        batch: &CellBatch,
        atlas: &ProtocolObject<dyn MTLTexture>,
        atlas_px: [f32; 2],
        sync: bool,
    ) {
        // 1. Block until a slot is available (≤ IN_FLIGHT frames in flight).
        {
            let (lock, cvar) = &*self.instance_semaphore;
            let mut count = lock.lock().unwrap();
            while *count <= 0 {
                count = cvar.wait(count).unwrap();
            }
            *count -= 1;
        }

        objc2::rc::autoreleasepool(|_| {
            // 2. Pick the next instance buffer (round-robin).
            let buf_idx = self.instance_buffer_idx;
            self.instance_buffer_idx = (self.instance_buffer_idx + 1) % IN_FLIGHT;

            let instance_bytes = batch.instance_bytes();
            let needed = instance_bytes.len();

            // 3. Grow the instance buffer if needed (next power of two).
            if needed > 0 {
                let current_len = self.instance_buffers[buf_idx].length();
                if needed > current_len {
                    let new_len = needed.next_power_of_two();
                    if let Some(new_buf) = make_buffer(&self.device, new_len) {
                        self.instance_buffers[buf_idx] = new_buf;
                    }
                    // If make_buffer failed, we proceed with the old buffer and
                    // clamp to its capacity (caller's batch was too large).
                }
            }

            // 4. Copy instance data into the buffer.
            if needed > 0 {
                let buf_len = self.instance_buffers[buf_idx].length();
                let copy_len = needed.min(buf_len);
                // SAFETY: contents() returns a non-null pointer into the
                // MTLBuffer's CPU-visible shared memory.  The buffer is
                // MTLResourceStorageModeShared so writes are immediately
                // visible to the GPU after the command buffer commits.
                let dst = self.instance_buffers[buf_idx].contents();
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        instance_bytes.as_ptr(),
                        dst.as_ptr() as *mut u8,
                        copy_len,
                    );
                }
            }

            // 5. Update the uniforms buffer.
            let uniforms = Uniforms {
                viewport_px: batch.viewport_px,
                atlas_px,
            };
            // SAFETY: uniforms is a plain POD struct; the buffer is shared storage.
            let udst = self.uniforms_buffer.contents();
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &uniforms as *const Uniforms as *const u8,
                    udst.as_ptr() as *mut u8,
                    std::mem::size_of::<Uniforms>(),
                );
            }

            // 6. Acquire drawable, build render pass descriptor.
            self.layer.setPresentsWithTransaction(sync);
            let drawable = match self.layer.nextDrawable() {
                Some(d) => d,
                None => {
                    // Return the semaphore slot we took.
                    let (lock, cvar) = &*self.instance_semaphore;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                    return;
                }
            };
            let dst_texture = drawable.texture();

            let rpd = MTLRenderPassDescriptor::renderPassDescriptor();
            let color_attachments = rpd.colorAttachments();
            // SAFETY: index 0 always exists on a freshly created descriptor.
            let att = unsafe { color_attachments.objectAtIndexedSubscript(0) };
            att.setTexture(Some(&dst_texture));
            att.setLoadAction(MTLLoadAction::Clear);
            att.setStoreAction(MTLStoreAction::Store);
            att.setClearColor(self.clear);

            // 7–10. Encode the draw call.
            let cmd = match self.queue.commandBuffer() {
                Some(c) => c,
                None => {
                    let (lock, cvar) = &*self.instance_semaphore;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                    return;
                }
            };
            let enc = match cmd.renderCommandEncoderWithDescriptor(&rpd) {
                Some(e) => e,
                None => {
                    let (lock, cvar) = &*self.instance_semaphore;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                    return;
                }
            };

            enc.setRenderPipelineState(&self.cell_pipeline);

            // SAFETY: buffer indices match the MSL bindings; all buffers are valid.
            unsafe {
                enc.setVertexBuffer_offset_atIndex(Some(&self.uniforms_buffer), 0, 1);
                enc.setVertexBuffer_offset_atIndex(Some(&self.instance_buffers[buf_idx]), 0, 2);
                enc.setFragmentBuffer_offset_atIndex(Some(&self.uniforms_buffer), 0, 1);
                enc.setFragmentTexture_atIndex(Some(atlas), 0);
            }

            let instance_count = batch.instance_count();
            if instance_count > 0 {
                // SAFETY: 6 vertices per instance, triangle primitive.
                unsafe {
                    enc.drawPrimitives_vertexStart_vertexCount_instanceCount(
                        MTLPrimitiveType::Triangle,
                        0,
                        6,
                        instance_count,
                    );
                }
            }
            enc.endEncoding();

            // 11. Signal the semaphore when the GPU finishes this command buffer.
            let sem = Arc::clone(&self.instance_semaphore);
            // SAFETY: the block is called exactly once by Metal after the command
            // buffer completes.  The Arc keeps the semaphore alive.
            let handler =
                RcBlock::new(move |_cmd: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    let (lock, cvar) = &*sem;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                });
            // SAFETY: handler is a valid block pointer; the block's lifetime
            // exceeds the command buffer's (Arc ensures the data outlives the call).
            unsafe {
                cmd.addCompletedHandler(RcBlock::as_ptr(&handler));
            }

            // 12. Present (sync or async, mirrors the CPU path).
            if sync {
                cmd.commit();
                cmd.waitUntilScheduled();
                let draw_ref: &ProtocolObject<dyn MTLDrawable> = unsafe {
                    &*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)
                };
                draw_ref.present();
            } else {
                let draw_ref: &ProtocolObject<dyn MTLDrawable> = unsafe {
                    &*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)
                };
                cmd.presentDrawable(draw_ref);
                cmd.commit();
            }
        });
    }

    /// Draw a full frame: first paint the chrome BGRA bitmap (CPU raster),
    /// then draw cell instances on top using the GPU cell pipeline.
    ///
    /// Phase C — called by `App::render_frame` when `ANVIL_RENDER=gpu`.
    ///
    /// `chrome_pixels` is the full-window BGRA8 raster produced by the CPU path
    /// (only chrome; viewport cells are skipped on the CPU side).
    /// `cells` is the `CellBatch` filled by `draw_viewport_gpu` for all visible
    /// panes.  `atlas` is the R8Unorm atlas texture from `AtlasPainter`.
    ///
    /// Single render pass, two sequential draws:
    /// 1. CPU pipeline full-screen quad paints the chrome bitmap.
    /// 2. Cell pipeline instanced draw paints the glyph cells on top (alpha blend).
    pub fn present_layered(
        &mut self,
        chrome_pixels: &[u8],
        cells: &CellBatch,
        atlas: &ProtocolObject<dyn MTLTexture>,
        atlas_px: [f32; 2],
        sync: bool,
    ) {
        // 1. Block until a slot is available (mirrors present_cells).
        {
            let (lock, cvar) = &*self.instance_semaphore;
            let mut count = lock.lock().unwrap();
            while *count <= 0 {
                count = cvar.wait(count).unwrap();
            }
            *count -= 1;
        }

        objc2::rc::autoreleasepool(|_| {
            // 2. Upload chrome pixels to the BGRA texture (same as present()).
            let region = MTLRegion {
                origin: MTLOrigin { x: 0, y: 0, z: 0 },
                size: MTLSize {
                    width: self.width,
                    height: self.height,
                    depth: 1,
                },
            };
            // SAFETY: chrome_pixels is a non-empty slice; pointer is non-null.
            let bytes_ptr = NonNull::new(chrome_pixels.as_ptr() as *mut std::ffi::c_void).unwrap();
            unsafe {
                self.texture
                    .replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                        region,
                        0,
                        bytes_ptr,
                        self.width * 4,
                    );
            }

            // 3. Pick the next instance buffer (round-robin) and copy cell data.
            let buf_idx = self.instance_buffer_idx;
            self.instance_buffer_idx = (self.instance_buffer_idx + 1) % IN_FLIGHT;

            let instance_bytes = cells.instance_bytes();
            let needed = instance_bytes.len();

            if needed > 0 {
                let current_len = self.instance_buffers[buf_idx].length();
                if needed > current_len {
                    let new_len = needed.next_power_of_two();
                    if let Some(new_buf) = make_buffer(&self.device, new_len) {
                        self.instance_buffers[buf_idx] = new_buf;
                    }
                }
                let buf_len = self.instance_buffers[buf_idx].length();
                let copy_len = needed.min(buf_len);
                // SAFETY: instance buffer is shared storage; dst pointer is valid.
                let dst = self.instance_buffers[buf_idx].contents();
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        instance_bytes.as_ptr(),
                        dst.as_ptr() as *mut u8,
                        copy_len,
                    );
                }
            }

            // 4. Update uniforms buffer.
            let uniforms = Uniforms {
                viewport_px: cells.viewport_px,
                atlas_px,
            };
            let udst = self.uniforms_buffer.contents();
            // SAFETY: Uniforms is a plain POD struct; buffer is shared storage.
            unsafe {
                std::ptr::copy_nonoverlapping(
                    &uniforms as *const Uniforms as *const u8,
                    udst.as_ptr() as *mut u8,
                    std::mem::size_of::<Uniforms>(),
                );
            }

            // 5. Acquire drawable.
            self.layer.setPresentsWithTransaction(sync);
            let drawable = match self.layer.nextDrawable() {
                Some(d) => d,
                None => {
                    let (lock, cvar) = &*self.instance_semaphore;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                    return;
                }
            };
            let dst_texture = drawable.texture();

            // 6. Build render pass descriptor.
            let rpd = MTLRenderPassDescriptor::renderPassDescriptor();
            let color_attachments = rpd.colorAttachments();
            // SAFETY: index 0 always exists on a freshly created descriptor.
            let att = unsafe { color_attachments.objectAtIndexedSubscript(0) };
            att.setTexture(Some(&dst_texture));
            att.setLoadAction(MTLLoadAction::Clear);
            att.setStoreAction(MTLStoreAction::Store);
            att.setClearColor(self.clear);

            // 7. Command buffer.
            let cmd = match self.queue.commandBuffer() {
                Some(c) => c,
                None => {
                    let (lock, cvar) = &*self.instance_semaphore;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                    return;
                }
            };
            let enc = match cmd.renderCommandEncoderWithDescriptor(&rpd) {
                Some(e) => e,
                None => {
                    let (lock, cvar) = &*self.instance_semaphore;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                    return;
                }
            };

            // 8. First draw: CPU pipeline paints the chrome BGRA bitmap fullscreen.
            enc.setRenderPipelineState(&self.pipeline);
            // SAFETY: fragment texture at index 0; texture is valid.
            unsafe {
                enc.setFragmentTexture_atIndex(Some(&self.texture), 0);
            }
            // SAFETY: 6 vertices, triangle primitive.
            unsafe {
                enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 6);
            }

            // 9. Second draw: cell pipeline paints glyph instances on top.
            let instance_count = cells.instance_count();
            if instance_count > 0 {
                enc.setRenderPipelineState(&self.cell_pipeline);
                // SAFETY: buffer indices match MSL bindings; all buffers valid.
                unsafe {
                    enc.setVertexBuffer_offset_atIndex(Some(&self.uniforms_buffer), 0, 1);
                    enc.setVertexBuffer_offset_atIndex(Some(&self.instance_buffers[buf_idx]), 0, 2);
                    enc.setFragmentBuffer_offset_atIndex(Some(&self.uniforms_buffer), 0, 1);
                    enc.setFragmentTexture_atIndex(Some(atlas), 0);
                }
                // SAFETY: 6 vertices per instance, triangle primitive.
                unsafe {
                    enc.drawPrimitives_vertexStart_vertexCount_instanceCount(
                        MTLPrimitiveType::Triangle,
                        0,
                        6,
                        instance_count,
                    );
                }
            }

            enc.endEncoding();

            // 10. Signal semaphore on GPU completion.
            let sem = Arc::clone(&self.instance_semaphore);
            // SAFETY: block is called exactly once by Metal after completion.
            let handler =
                RcBlock::new(move |_cmd: NonNull<ProtocolObject<dyn MTLCommandBuffer>>| {
                    let (lock, cvar) = &*sem;
                    *lock.lock().unwrap() += 1;
                    cvar.notify_one();
                });
            // SAFETY: handler is a valid block pointer; Arc keeps data alive.
            unsafe {
                cmd.addCompletedHandler(RcBlock::as_ptr(&handler));
            }

            // 11. Present (sync or async, mirrors the CPU path).
            if sync {
                cmd.commit();
                cmd.waitUntilScheduled();
                let draw_ref: &ProtocolObject<dyn MTLDrawable> = unsafe {
                    &*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)
                };
                draw_ref.present();
            } else {
                let draw_ref: &ProtocolObject<dyn MTLDrawable> = unsafe {
                    &*(Retained::as_ptr(&drawable) as *const ProtocolObject<dyn MTLDrawable>)
                };
                cmd.presentDrawable(draw_ref);
                cmd.commit();
            }
        });
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn rgb_clear_color(r: u8, g: u8, b: u8) -> MTLClearColor {
    MTLClearColor {
        red: r as f64 / 255.0,
        green: g as f64 / 255.0,
        blue: b as f64 / 255.0,
        alpha: 1.0,
    }
}

fn make_texture(
    device: &ProtocolObject<dyn MTLDevice>,
    width: usize,
    height: usize,
) -> Result<Retained<ProtocolObject<dyn MTLTexture>>, RendererError> {
    // SAFETY: texture2DDescriptorWithPixelFormat_width_height_mipmapped is a
    // class method that returns a +1 retained descriptor.
    let desc = unsafe {
        MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
            PIXEL_FORMAT,
            width,
            height,
            false,
        )
    };
    let tex = device
        .newTextureWithDescriptor(&desc)
        .ok_or(RendererError::TextureFailed)?;
    Ok(tex)
}

fn make_buffer(
    device: &ProtocolObject<dyn MTLDevice>,
    len: usize,
) -> Option<Retained<ProtocolObject<dyn MTLBuffer>>> {
    // MTLResourceStorageModeShared: CPU-writable, GPU-readable without a blit.
    device.newBufferWithLength_options(len, MTLResourceOptions::StorageModeShared)
}

fn build_cell_pipeline(
    device: &ProtocolObject<dyn MTLDevice>,
) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>, RendererError> {
    let src = NSString::from_str(CELL_SHADER_SRC);
    let library = device
        .newLibraryWithSource_options_error(&src, None)
        .map_err(|e| RendererError::ShaderCompileFailed(format!("{}", e)))?;

    let vname = NSString::from_str("cell_v");
    let fname = NSString::from_str("cell_f");

    let vfn: Retained<ProtocolObject<dyn MTLFunction>> = library
        .newFunctionWithName(&vname)
        .ok_or_else(|| RendererError::ShaderCompileFailed("cell_v not found".into()))?;
    let ffn: Retained<ProtocolObject<dyn MTLFunction>> = library
        .newFunctionWithName(&fname)
        .ok_or_else(|| RendererError::ShaderCompileFailed("cell_f not found".into()))?;

    let desc = MTLRenderPipelineDescriptor::new();
    desc.setVertexFunction(Some(&vfn));
    desc.setFragmentFunction(Some(&ffn));
    // SAFETY: index 0 always exists on a freshly created descriptor.
    unsafe {
        let att = desc.colorAttachments().objectAtIndexedSubscript(0);
        att.setPixelFormat(PIXEL_FORMAT);
        // Enable src-alpha blending so cell glyphs compose over whatever the
        // chrome pass painted underneath (used by present_layered).
        att.setBlendingEnabled(true);
        att.setSourceRGBBlendFactor(MTLBlendFactor::SourceAlpha);
        att.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        att.setRgbBlendOperation(MTLBlendOperation::Add);
        att.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        att.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        att.setAlphaBlendOperation(MTLBlendOperation::Add);
    }

    let pipeline = device
        .newRenderPipelineStateWithDescriptor_error(&desc)
        .map_err(|e| RendererError::PipelineCreationFailed(format!("{}", e)))?;
    Ok(pipeline)
}

fn build_pipeline(
    device: &ProtocolObject<dyn MTLDevice>,
) -> Result<Retained<ProtocolObject<dyn MTLRenderPipelineState>>, RendererError> {
    let src = NSString::from_str(SHADER_SRC);

    let library = device
        .newLibraryWithSource_options_error(&src, None)
        .map_err(|e| RendererError::ShaderCompileFailed(format!("{}", e)))?;

    let vname = NSString::from_str("v_main");
    let fname = NSString::from_str("f_main");

    let vfn: Retained<ProtocolObject<dyn MTLFunction>> = library
        .newFunctionWithName(&vname)
        .ok_or_else(|| RendererError::ShaderCompileFailed("v_main not found".into()))?;

    let ffn: Retained<ProtocolObject<dyn MTLFunction>> = library
        .newFunctionWithName(&fname)
        .ok_or_else(|| RendererError::ShaderCompileFailed("f_main not found".into()))?;

    let desc = MTLRenderPipelineDescriptor::new();
    desc.setVertexFunction(Some(&vfn));
    desc.setFragmentFunction(Some(&ffn));
    // SAFETY: index 0 always exists on a freshly created descriptor.
    unsafe {
        desc.colorAttachments()
            .objectAtIndexedSubscript(0)
            .setPixelFormat(PIXEL_FORMAT);
    }

    let pipeline = device
        .newRenderPipelineStateWithDescriptor_error(&desc)
        .map_err(|e| RendererError::PipelineCreationFailed(format!("{}", e)))?;

    Ok(pipeline)
}

// ── tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// presentMode returns sync during live resize and async otherwise
    #[test]
    fn present_mode_sync_during_resize_async_otherwise() {
        assert_eq!(present_mode(true), PresentMode::Sync);
        assert_eq!(present_mode(false), PresentMode::Async);
    }

    // ── Phase B tests (macOS only) ────────────────────────────────────────────

    /// `build_cell_pipeline` compiles the MSL and creates the pipeline state
    /// without error when a Metal device is available.
    ///
    /// We call `build_cell_pipeline` directly rather than `Renderer::init`
    /// because constructing a `Renderer` requires a `CAMetalLayer` with an
    /// attached display connection, which is not available in headless test
    /// environments.
    #[cfg(target_os = "macos")]
    #[test]
    fn cell_pipeline_state_creates_without_error() {
        let Some(device) = MTLCreateSystemDefaultDevice() else {
            // No Metal device available in this environment; skip silently.
            return;
        };
        let result = build_cell_pipeline(&device);
        assert!(
            result.is_ok(),
            "cell pipeline creation failed: {:?}",
            result.err()
        );
    }

    /// Verifies that `make_buffer` allocates a non-zero MTL buffer, and that
    /// `CellBatch::instance_bytes()` can be written into it without panicking.
    ///
    /// NOTE: `present_cells` requires `CAMetalLayer::nextDrawable`, which
    /// cannot be called in a headless test environment (no display connection).
    /// We therefore test the buffer allocation and data-copy path in isolation
    /// rather than doing a full end-to-end render + readback.
    #[cfg(target_os = "macos")]
    #[test]
    fn present_cells_smoke_buffer_copy() {
        use anvil_render::{CellBatch, GlyphSlot};

        let Some(device) = MTLCreateSystemDefaultDevice() else {
            return;
        };

        // Build a CellBatch with 10 cells (mix of glyph and bg-only).
        let mut batch = CellBatch::new();
        batch.clear([800.0, 600.0]);
        for i in 0..10usize {
            let slot = if i % 2 == 0 {
                Some(GlyphSlot {
                    atlas_x: (i * 12) as u16,
                    atlas_y: 0,
                    w: 10,
                    h: 20,
                    bearing_x: 1,
                    bearing_y: 2,
                })
            } else {
                None
            };
            batch.push_cell(
                [(i as f32) * 12.0, 0.0],
                [12.0, 24.0],
                slot,
                [255, 255, 255],
                [0, 0, 0],
            );
        }
        assert_eq!(batch.instance_count(), 10);

        let instance_bytes = batch.instance_bytes();
        let needed = instance_bytes.len();
        assert!(needed > 0);

        // Allocate a buffer and copy.
        let buf = make_buffer(&device, needed).expect("make_buffer must succeed");
        assert!(buf.length() >= needed);

        let dst = buf.contents();
        // SAFETY: buf is a shared-storage buffer; dst is a valid non-null pointer.
        unsafe {
            std::ptr::copy_nonoverlapping(instance_bytes.as_ptr(), dst.as_ptr() as *mut u8, needed);
        }

        // Read back the first byte to confirm the copy landed.
        let first_byte = unsafe { *(dst.as_ptr() as *const u8) };
        // cell_px_xy[0] for cell 0 is 0.0f32 → bytes [0x00, 0x00, 0x00, 0x00].
        assert_eq!(first_byte, 0x00);
    }
}
