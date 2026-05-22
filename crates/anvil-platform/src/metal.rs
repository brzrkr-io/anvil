//! Metal renderer.
//!
//! The terminal is rasterized on the CPU into a BGRA8 bitmap each frame; this
//! module uploads that bitmap to a `MTLTexture` and draws it as a single
//! full-screen quad via a runtime-compiled MSL shader (no offline `metal`
//! toolchain required).
//!
//! Port of `src/render/metal.zig`.  Uses `objc2-metal` 0.3 typed bindings.
//! Unsafe blocks are marked with a SAFETY comment explaining the invariant.

use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use objc2_metal::{
    MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLFunction, MTLLibrary, MTLLoadAction,
    MTLOrigin, MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder,
    MTLRenderPassDescriptor, MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLSize,
    MTLStoreAction, MTLTexture, MTLTextureDescriptor,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use thiserror::Error;

const PIXEL_FORMAT: MTLPixelFormat = MTLPixelFormat::BGRA8Unorm;

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
pub struct Renderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    layer: Retained<CAMetalLayer>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    width: usize,
    height: usize,
    clear: MTLClearColor,
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

    /// Port of "presentMode returns sync during live resize and async otherwise"
    #[test]
    fn present_mode_sync_during_resize_async_otherwise() {
        assert_eq!(present_mode(true), PresentMode::Sync);
        assert_eq!(present_mode(false), PresentMode::Async);
    }
}
