// What this GPU and browser allow and are, for JS: the limits the library
// checks against, and the vendor / renderer strings.

use wasm_bindgen::prelude::*;
use web_sys::WebGl2RenderingContext as Gl;

use crate::context::Context;
use crate::limits;

#[wasm_bindgen(typescript_custom_section)]
const TS_LIMITS: &str = r#"
/** What this GPU and browser allow (`ctx.limits()`), as WebGL reports them. */
export interface ContextLimits {
  maxTextureSize: number;
  max3dTextureSize: number;
  maxArrayTextureLayers: number;
  maxCubeMapTextureSize: number;
  maxRenderbufferSize: number;
  maxSamples: number;
  maxColorAttachments: number;
  maxDrawBuffers: number;
  maxTextureImageUnits: number;
  maxCombinedTextureUnits: number;
  maxVertexAttribs: number;
  maxUniformBlockSize: number;
  maxUniformBufferBindings: number;
  /** Offsets of `paramSetBlockRange` ranges are multiples of this. */
  uniformBufferOffsetAlignment: number;
}

/** The GPU, as far as the browser tells (`ctx.gpuInfo()`). */
export interface GpuInfo {
  /** `VENDOR` / `RENDERER`: often generic ("WebKit", "WebKit WebGL"). */
  vendor: string;
  renderer: string;
  /** The real names where `WEBGL_debug_renderer_info` gives them (e.g.
   *  "ANGLE (NVIDIA, …)"); else the same as above. */
  unmaskedVendor: string;
  unmaskedRenderer: string;
}
"#;

// WEBGL_debug_renderer_info's UNMASKED_VENDOR_WEBGL / UNMASKED_RENDERER_WEBGL.
const UNMASKED_VENDOR: u32 = 0x9245;
const UNMASKED_RENDERER: u32 = 0x9246;

#[wasm_bindgen]
impl Context {
    /// What this GPU and browser allow (the values the library checks
    /// against): texture and renderbuffer sizes, samples, attachments,
    /// texture units, uniform block sizes and offset alignment.
    #[wasm_bindgen(js_name = limits, unchecked_return_type = "ContextLimits")]
    pub fn limits_js(&self) -> JsValue {
        let o = js_sys::Object::new();
        let set = |k: &str, v: u32| {
            let _ = js_sys::Reflect::set(&o, &k.into(), &v.into());
        };
        set("maxTextureSize", limits::max_texture_size());
        set("max3dTextureSize", limits::max_3d_texture_size());
        set("maxArrayTextureLayers", limits::max_array_texture_layers());
        set("maxCubeMapTextureSize", limits::max_cube_map_texture_size());
        set("maxRenderbufferSize", limits::max_renderbuffer_size());
        set("maxSamples", limits::max_samples());
        set("maxColorAttachments", limits::max_color_attachments());
        set("maxDrawBuffers", limits::max_draw_buffers());
        set("maxTextureImageUnits", limits::max_texture_image_units());
        set(
            "maxCombinedTextureUnits",
            limits::max_combined_texture_units(),
        );
        set("maxVertexAttribs", limits::max_vertex_attribs());
        set("maxUniformBlockSize", limits::max_uniform_block_size());
        set(
            "maxUniformBufferBindings",
            limits::max_uniform_buffer_bindings(),
        );
        set(
            "uniformBufferOffsetAlignment",
            limits::uniform_buffer_offset_alignment(),
        );
        o.into()
    }

    /// The GPU's vendor and renderer strings (for logs, bug reports, picking
    /// defaults per device): the masked `VENDOR` / `RENDERER`, and the
    /// unmasked ones where `WEBGL_debug_renderer_info` is offered.
    #[wasm_bindgen(js_name = gpuInfo, unchecked_return_type = "GpuInfo")]
    pub fn gpu_info(&self) -> JsValue {
        let gl = self.gl();
        let text = |pname: u32| {
            gl.get_parameter(pname)
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        let (vendor, renderer) = (text(Gl::VENDOR), text(Gl::RENDERER));
        let unmasked = matches!(
            gl.get_extension(crate::extension::Extension::WebglDebugRendererInfo.name()),
            Ok(Some(_))
        );
        let (uv, ur) = if unmasked {
            (text(UNMASKED_VENDOR), text(UNMASKED_RENDERER))
        } else {
            (vendor.clone(), renderer.clone())
        };
        let o = js_sys::Object::new();
        for (k, v) in [
            ("vendor", vendor),
            ("renderer", renderer),
            ("unmaskedVendor", uv),
            ("unmaskedRenderer", ur),
        ] {
            let _ = js_sys::Reflect::set(&o, &k.into(), &v.into());
        }
        o.into()
    }
}
