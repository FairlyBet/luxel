//! Screenshots: a frame drawn as `renderFrame` draws it (HDR, antialiasing,
//! post), at a size of the caller's, encoded by the browser.

use rswebgl::draw::Viewport;
use rswebgl::error::Error;
use rswebgl_ecs::Entity;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use super::{Scene, options};

#[wasm_bindgen(typescript_custom_section)]
const TS_CAPTURE: &str = r#"
/** How `capture` takes a picture. */
export interface CaptureOptions {
  /** Size in pixels; default the canvas's drawing buffer. Clamped to what
   *  the GPU renders. */
  width?: number;
  height?: number;
  /** "image/png" (default), "image/jpeg" or "image/webp". */
  type?: string;
  /** 0–1 for JPEG / WebP. */
  quality?: number;
  /** Frames drawn before the picture: TAA converges over them (default 30
   *  with TAA, else 1). */
  frames?: number;
}
"#;

#[wasm_bindgen]
impl Scene {
    /// A picture of the scene from `camera`, drawn as `renderFrame` draws it
    /// (HDR, antialiasing, effects; the canvas's background and clear
    /// color), at `options.width` × `options.height`: a promise of the
    /// encoded image. The canvas's drawing buffer takes that size for the
    /// moment (its CSS size stays) and the canvas is drawn again at its own
    /// size after. At most 8192 a side (and what the GPU and browser allow:
    /// a smaller buffer comes out stretched); memory grows with the size —
    /// HDR, MSAA and TAA targets at that size for the moment, which a phone
    /// may not have. Rejects without a canvas or with a lost context.
    #[wasm_bindgen(unchecked_return_type = "Promise<Blob>")]
    pub fn capture(
        &mut self,
        camera: Entity,
        #[wasm_bindgen(unchecked_param_type = "CaptureOptions | undefined")] opts: Option<JsValue>,
    ) -> Result<js_sys::Promise, Error> {
        let opts = opts.unwrap_or(JsValue::UNDEFINED);
        let fb = self.ctx.default_framebuffer();
        let canvas = fb.canvas().ok_or("capture: the context has no canvas")?;
        if self.ctx.is_lost() {
            return Err(Error::context_lost("capture: the WebGL context is lost"));
        }
        let (old_w, old_h) = (canvas.width(), canvas.height());
        let max = rswebgl::limits::max_renderbuffer_size().clamp(1, 8192);
        let size = |key: &str, default: u32| -> Result<u32, Error> {
            Ok(options::u32_or(&opts, key, default)?.clamp(1, max))
        };
        let (w, h) = (size("width", old_w.max(1))?, size("height", old_h.max(1))?);
        let mime = options::string_or(&opts, "type", "image/png")?;
        let quality = match options::f32_or(&opts, "quality", f32::NAN)? {
            q if q.is_nan() => JsValue::UNDEFINED,
            q => q.clamp(0.0, 1.0).into(),
        };
        let taa = self.taa();
        let frames = options::u32_or(&opts, "frames", if taa { super::on_demand::TAA_SETTLE } else { 1 })?.clamp(1, 256);

        let resized = (w, h) != (old_w, old_h);
        if resized {
            canvas.set_width(w);
            canvas.set_height(h);
            set_viewport_to_buffer(&fb);
        }
        for _ in 0..frames {
            self.render_frame(camera);
        }
        // The drawing buffer is read now, before it's shown (and cleared):
        // toBlob copies the bitmap at once and encodes later.
        let mut started = Ok(());
        let promise = js_sys::Promise::new(&mut |resolve, reject| {
            let done = Closure::once_into_js(move |blob: JsValue| {
                if blob.is_null() {
                    let _ = reject.call1(
                        &JsValue::NULL,
                        &js_sys::Error::new("capture: the image couldn't be encoded").into(),
                    );
                } else {
                    let _ = resolve.call1(&JsValue::NULL, &blob);
                }
            });
            started =
                canvas.to_blob_with_type_and_encoder_options(done.unchecked_ref(), &mime, &quality);
        });
        if resized {
            canvas.set_width(old_w);
            canvas.set_height(old_h);
            set_viewport_to_buffer(&fb);
            // A resize cleared the canvas, and the camera has the picture's
            // aspect: a frame at the canvas's size now (TAA settles on).
            self.render_frame(camera);
            self.invalidate();
        }
        started.map_err(|e| Error::from(format!("capture: {e:?}")))?;
        Ok(promise)
    }
}

// The viewport over the drawing buffer the browser gave (it may be smaller
// than asked).
fn set_viewport_to_buffer(fb: &rswebgl::framebuffer::DefaultFramebuffer) {
    let size = fb.drawing_buffer_size();
    fb.set_viewport(Viewport::new(0, 0, size[0].max(1), size[1].max(1)));
}
