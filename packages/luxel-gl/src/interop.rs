// Raw GL interop: the WebGL2 context and the GL objects behind handles, for
// sharing them with other code (another engine, a video pipeline, WebXR),
// and telling the context when that code changed GL state.

use wasm_bindgen::prelude::*;
use web_sys::{WebGl2RenderingContext, WebGlBuffer, WebGlFramebuffer, WebGlProgram, WebGlTexture};

use crate::context::Context;
use crate::ids::{BufferId, FramebufferId, ProgramId, TextureId};

#[wasm_bindgen]
impl Context {
    /// The WebGL2 context underneath, for code of your own. GL calls made on
    /// it change state the context caches: call `resetGlState` after them,
    /// before the next `render`.
    #[wasm_bindgen(js_name = rawGl)]
    pub fn raw_gl(&self) -> WebGl2RenderingContext {
        self.gl().clone()
    }

    /// Forgets the GL state the context believes is set (bindings,
    /// program, vertex array, render state, texture units): the next render
    /// sets all it needs again. Call after GL calls of your own on `rawGl`
    /// (or another library sharing the context).
    #[wasm_bindgen(js_name = resetGlState)]
    pub fn reset_gl_state(&self) {
        self.invalidate_state();
        // State the library relies on being at its default and doesn't
        // cache: buffers bound for pixel transfers, pack parameters,
        // switches outside render states.
        let gl = self.gl();
        type Gl = WebGl2RenderingContext;
        gl.bind_buffer(Gl::PIXEL_PACK_BUFFER, None);
        gl.bind_buffer(Gl::PIXEL_UNPACK_BUFFER, None);
        gl.pixel_storei(Gl::PACK_ALIGNMENT, 4);
        gl.pixel_storei(Gl::PACK_ROW_LENGTH, 0);
        gl.pixel_storei(Gl::PACK_SKIP_ROWS, 0);
        gl.pixel_storei(Gl::PACK_SKIP_PIXELS, 0);
        gl.disable(Gl::RASTERIZER_DISCARD);
        gl.disable(Gl::SAMPLE_ALPHA_TO_COVERAGE);
        gl.disable(Gl::SAMPLE_COVERAGE);
        // Reverse-Z's clip control and depth range, as the context set them.
        crate::depth_range::apply(&self.core);
    }

    /// The `WebGLTexture` behind `tex` (undefined for a stale handle): to
    /// sample or fill from other code. Don't delete it; `destroyTexture` does.
    #[wasm_bindgen(js_name = rawTexture)]
    pub fn raw_texture(&self, tex: TextureId) -> Option<WebGlTexture> {
        self.res().textures.get(tex.0).map(|t| t.raw.clone())
    }

    /// The `WebGLBuffer` behind `buffer` (undefined for a stale handle).
    #[wasm_bindgen(js_name = rawBuffer)]
    pub fn raw_buffer(&self, buffer: BufferId) -> Option<WebGlBuffer> {
        self.res().buffers.get(buffer.0).map(|b| b.raw.clone())
    }

    /// The `WebGLFramebuffer` behind `fb` (undefined for a stale handle or
    /// the canvas, NULL).
    #[wasm_bindgen(js_name = rawFramebuffer)]
    pub fn raw_framebuffer(&self, fb: FramebufferId) -> Option<WebGlFramebuffer> {
        self.res().framebuffers.get(fb.0).map(|f| f.raw.clone())
    }

    /// The `WebGLProgram` behind `program` (undefined for a stale handle).
    /// Its uniforms are the context's to set: values written to it from
    /// outside aren't known to its cache, which may skip writing them again.
    #[wasm_bindgen(js_name = rawProgram)]
    pub fn raw_program(&self, program: ProgramId) -> Option<WebGlProgram> {
        self.res().programs.get(program.0).map(|p| p.raw.clone())
    }
}
