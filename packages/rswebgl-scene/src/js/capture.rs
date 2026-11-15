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
    /// moment (its CSS size stays) and is redrawn after (`needsRender`).
    /// Rejects without a canvas or with a lost context.
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
            return Err("capture: the WebGL context is lost".into());
        }
        let (old_w, old_h) = (canvas.width(), canvas.height());
        let max = rswebgl::limits::max_renderbuffer_size().max(1);
        let size = |key: &str, default: u32| -> Result<u32, Error> {
            Ok(options::u32_or(&opts, key, default)?.clamp(1, max))
        };
        let (w, h) = (size("width", old_w.max(1))?, size("height", old_h.max(1))?);
        let mime = options::string_or(&opts, "type", "image/png")?;
        let quality = match options::f32_or(&opts, "quality", f32::NAN)? {
            q if q.is_nan() => JsValue::UNDEFINED,
            q => q.clamp(0.0, 1.0).into(),
        };
        let taa = self.taa && self.hdr_wanted;
        let frames = options::u32_or(&opts, "frames", if taa { 30 } else { 1 })?.clamp(1, 256);

        let resized = (w, h) != (old_w, old_h);
        if resized {
            canvas.set_width(w);
            canvas.set_height(h);
            fb.set_viewport(Viewport::new(0, 0, w as i32, h as i32));
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
            fb.set_viewport(Viewport::new(0, 0, old_w as i32, old_h as i32));
        }
        // What the canvas showed is gone (a resize clears it): drawn again.
        self.invalidate();
        started.map_err(|e| Error::from(format!("capture: {e:?}")))?;
        Ok(promise)
    }
}
