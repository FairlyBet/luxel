// GPU-side copies: framebuffer blits (MSAA resolve, scaling), buffer to
// buffer, framebuffer to texture. Nothing passes through the CPU.
//
// These bind READ / DRAW framebuffers and copy targets outside the renderer, so
// each tells the state cache what it disturbed. The common case — resolving an
// MSAA pass into a texture or the canvas — is a pass store op
// (`Pass::set_resolve`), done by the renderer right after the pass's draws.
//
// Like other operations on existing objects, a rejected copy is reported
// (console.error) and does nothing.

use wasm_bindgen::prelude::*;
use web_sys::{WebGl2RenderingContext as Gl, WebGlFramebuffer};

use crate::buffer::BufferKind;
use crate::console;
use crate::context::Context;
use crate::draw::Viewport;
use crate::framebuffer::ClearMask;
use crate::ids::{BufferId, FramebufferId, TextureId};
use crate::readback::Region;
use crate::texture::TextureMagFilter;

/// `blitFramebuffer` with the read buffer set to `src`'s color attachment 0
/// (the back buffer for the canvas). Leaves READ = `src`, DRAW = `dst` bound.
#[allow(clippy::too_many_arguments)]
pub(crate) fn blit(
    gl: &Gl,
    src: Option<&WebGlFramebuffer>,
    dst: Option<&WebGlFramebuffer>,
    s: Viewport,
    d: Viewport,
    mask: u32,
    filter: u32,
) {
    gl.bind_framebuffer(Gl::READ_FRAMEBUFFER, src);
    gl.read_buffer(if src.is_some() {
        Gl::COLOR_ATTACHMENT0
    } else {
        Gl::BACK
    });
    gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, dst);
    blit_unscissored(gl, s, d, mask, filter);
}

// The bound READ into the bound DRAW. blitFramebuffer obeys the scissor
// test: a draw's leftover scissor would clip the copy. Off for it, back as
// it was (the state cache stays right).
fn blit_unscissored(gl: &Gl, s: Viewport, d: Viewport, mask: u32, filter: u32) {
    let scissored = gl.is_enabled(Gl::SCISSOR_TEST);
    if scissored {
        gl.disable(Gl::SCISSOR_TEST);
    }
    gl.blit_framebuffer(
        s.x,
        s.y,
        s.x + s.width,
        s.y + s.height,
        d.x,
        d.y,
        d.x + d.width,
        d.y + d.height,
        mask,
        filter,
    );
    if scissored {
        gl.enable(Gl::SCISSOR_TEST);
    }
}

/// `blit` of one color attachment into one: `src`'s `src_index` read, only
/// `dst`'s `dst_index` written (the canvas: index 0, its back buffer).
/// `restore_dst` restores `dst`'s draw buffers after (`None` for the canvas).
#[allow(clippy::too_many_arguments)]
pub(crate) fn blit_attachment(
    gl: &Gl,
    src: Option<&WebGlFramebuffer>,
    src_index: u32,
    dst: Option<&WebGlFramebuffer>,
    dst_index: u32,
    s: Viewport,
    d: Viewport,
    filter: u32,
    restore_dst: Option<&dyn Fn(&Gl)>,
) {
    gl.bind_framebuffer(Gl::READ_FRAMEBUFFER, src);
    gl.read_buffer(if src.is_some() {
        Gl::COLOR_ATTACHMENT0 + src_index
    } else {
        Gl::BACK
    });
    gl.bind_framebuffer(Gl::DRAW_FRAMEBUFFER, dst);
    if dst.is_some() {
        let arr = js_sys::Array::new();
        for i in 0..=dst_index {
            let v = if i == dst_index {
                Gl::COLOR_ATTACHMENT0 + i
            } else {
                Gl::NONE
            };
            arr.push(&JsValue::from(v));
        }
        gl.draw_buffers(arr.as_ref());
    }
    blit_unscissored(gl, s, d, Gl::COLOR_BUFFER_BIT, filter);
    if let Some(restore) = restore_dst {
        restore(gl);
    }
    // READ_BUFFER back to attachment 0 (what a plain blit and readPixels
    // expect).
    if src.is_some() {
        gl.read_buffer(Gl::COLOR_ATTACHMENT0);
    }
}

/// Whether the canvas's drawing buffer is multisampled (antialias): a blit
/// can't write into it then (GL ES 3.0: DRAW with SAMPLE_BUFFERS > 0).
pub(crate) fn canvas_multisampled(gl: &Gl) -> bool {
    crate::framebuffer::context_attribute(gl, "antialias").unwrap_or(false)
}

#[wasm_bindgen]
impl Context {
    /// Copies `src_rect` of `src` into `dst_rect` of `dst`
    /// (`FramebufferId::NULL` = the canvas), scaling if the sizes differ.
    /// Color reads `src`'s attachment 0 and writes every color attachment of
    /// `dst`; depth and stencil copy as is and need `Nearest`. From a
    /// multisampled `src` this is the resolve: rects must then be the same
    /// size (see also `Pass::set_resolve`).
    #[wasm_bindgen(js_name = blitFramebuffer)]
    pub fn blit_framebuffer(
        &self,
        src: FramebufferId,
        src_rect: &Viewport,
        dst: FramebufferId,
        dst_rect: &Viewport,
        mask: &ClearMask,
        filter: TextureMagFilter,
    ) {
        const OP: &str = "blit_framebuffer";
        if *mask == ClearMask::none() {
            return;
        }
        if (mask.depth || mask.stencil) && filter != TextureMagFilter::Nearest {
            return console::error(&format!(
                "[luxel] {OP}: depth / stencil copy needs Nearest filtering; ignored"
            ));
        }
        let gl = self.gl();
        let res = self.res();
        let raw = |fb: FramebufferId| -> Result<Option<&WebGlFramebuffer>, ()> {
            if fb.is_null() {
                return Ok(None);
            }
            match res.framebuffers.get(fb.0) {
                Some(f) => Ok(Some(&f.raw)),
                None => {
                    self.stale("framebuffer", OP, fb);
                    Err(())
                }
            }
        };
        let (Ok(s), Ok(d)) = (raw(src), raw(dst)) else {
            return;
        };
        if mask.color
            && !src.is_null()
            && !res.framebuffers.get(src.0).is_some_and(|f| f.has_color(0))
        {
            return console::error(&format!(
                "[luxel] {OP}: source has no color attachment 0; ignored"
            ));
        }
        let samples = |fb: FramebufferId| {
            res.framebuffers
                .get(fb.0)
                .and_then(|f| f.color_format(&res, 0))
                .map_or(0, |(_, n)| n)
        };
        if !dst.is_null() && samples(dst) > 0 {
            return console::error(&format!(
                "[luxel] {OP}: the destination is multisampled (only reading from one \
                 resolves); ignored"
            ));
        }
        if dst.is_null() && canvas_multisampled(gl) {
            return console::error(&format!(
                "[luxel] {OP}: the canvas is multisampled (ContextOptions.antialias), \
                 a blit can't write into it; ignored"
            ));
        }
        // A multisampled source (a framebuffer's, or the antialiased
        // canvas) resolves only into the same rectangle: no scaling, no
        // offset (GLES 3.0 blitFramebuffer).
        let multisampled = if src.is_null() {
            canvas_multisampled(gl)
        } else {
            samples(src) > 0
        };
        if multisampled && src_rect != dst_rect {
            return console::error(&format!(
                "[luxel] {OP}: a multisampled source copies into the same rectangle only \
                 (no scaling or offset); ignored"
            ));
        }
        self.gl_state().forget_target();
        blit(gl, s, d, *src_rect, *dst_rect, mask.as_gl(), filter as u32);
    }

    /// `blit_framebuffer` of one color attachment: `src`'s `src_index` into
    /// `dst`'s `dst_index` only (`FramebufferId::NULL` = the canvas, index
    /// 0). From a multisampled `src` the resolve of that attachment (same
    /// rectangles then); the other attachments of `dst` are left alone.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = blitFramebufferAttachment)]
    pub fn blit_framebuffer_attachment(
        &self,
        src: FramebufferId,
        src_index: u32,
        src_rect: &Viewport,
        dst: FramebufferId,
        dst_index: u32,
        dst_rect: &Viewport,
        filter: TextureMagFilter,
    ) {
        const OP: &str = "blit_framebuffer_attachment";
        let gl = self.gl();
        let res = self.res();
        let get = |fb: FramebufferId| -> Result<Option<&crate::framebuffer::FramebufferObj>, ()> {
            if fb.is_null() {
                return Ok(None);
            }
            res.framebuffers
                .get(fb.0)
                .map(Some)
                .ok_or_else(|| self.stale("framebuffer", OP, fb))
        };
        let (Ok(s), Ok(d)) = (get(src), get(dst)) else {
            return;
        };
        let problem = if s.is_some_and(|f| !f.has_color(src_index))
            || (s.is_none() && src_index != 0)
        {
            Some(format!("the source has no color attachment {src_index}"))
        } else if d.is_some_and(|f| !f.has_color(dst_index)) || (d.is_none() && dst_index != 0) {
            Some(format!(
                "the destination has no color attachment {dst_index}"
            ))
        } else if d.is_none() && canvas_multisampled(gl) {
            Some(
                "the canvas is multisampled (ContextOptions.antialias), a blit can't write into it"
                    .into(),
            )
        } else if d
            .and_then(|f| f.color_format(&res, dst_index))
            .is_some_and(|(_, n)| n > 0)
        {
            Some("the destination is multisampled (only reading from one resolves)".into())
        } else if s
            .and_then(|f| f.color_format(&res, src_index))
            .is_some_and(|(_, n)| n > 0)
            && src_rect != dst_rect
        {
            Some("a multisampled source copies into the same rectangle only".into())
        } else if let (Some((a, n)), Some((b, _))) = (
            s.and_then(|f| f.color_format(&res, src_index)),
            d.and_then(|f| f.color_format(&res, dst_index)),
        ) && n > 0
            && a != b
        {
            Some(format!(
                "a multisampled source (0x{a:X}) resolves into the same format only (0x{b:X})"
            ))
        } else {
            None
        };
        if let Some(p) = problem {
            return console::error(&format!("[luxel] {OP}: {p}; ignored"));
        }
        self.gl_state().forget_target();
        let restore = d.map(|f| move |gl: &Gl| crate::framebuffer::sync_draw_buffers(f, gl));
        blit_attachment(
            gl,
            s.map(|f| &f.raw),
            src_index,
            d.map(|f| &f.raw),
            dst_index,
            *src_rect,
            *dst_rect,
            filter as u32,
            restore.as_ref().map(|r| r as &dyn Fn(&Gl)),
        );
    }

    /// Copies `size` bytes from `src` at `src_offset` to `dst` at `dst_offset`
    /// (`copyBufferSubData`). Index buffers can't take part: WebGL keeps them
    /// off the copy targets.
    #[wasm_bindgen(js_name = copyBuffer)]
    pub fn copy_buffer(
        &self,
        src: BufferId,
        src_offset: u32,
        dst: BufferId,
        dst_offset: u32,
        size: u32,
    ) {
        const OP: &str = "copy_buffer";
        let gl = self.gl();
        let res = self.res();
        let Some(s) = res.buffers.get(src.0) else {
            return self.stale("buffer", OP, src);
        };
        let Some(d) = res.buffers.get(dst.0) else {
            return self.stale("buffer", OP, dst);
        };
        let problem = if s.kind == BufferKind::Index || d.kind == BufferKind::Index {
            Some("index buffers can't be copied".to_string())
        } else if src_offset as u64 + size as u64 > s.size as u64
            || dst_offset as u64 + size as u64 > d.size as u64
        {
            Some(format!(
                "{size} bytes from {src_offset} / to {dst_offset} overrun the buffers \
                 ({} / {} bytes)",
                s.size, d.size
            ))
        } else if src == dst && src_offset < dst_offset + size && dst_offset < src_offset + size {
            Some("source and destination ranges overlap".to_string())
        } else {
            None
        };
        if let Some(p) = problem {
            return console::error(&format!("[luxel] {OP}: {p}; ignored"));
        }
        gl.bind_buffer(Gl::COPY_READ_BUFFER, Some(&s.raw));
        gl.bind_buffer(Gl::COPY_WRITE_BUFFER, Some(&d.raw));
        gl.copy_buffer_sub_data_with_i32_and_i32_and_i32(
            Gl::COPY_READ_BUFFER,
            Gl::COPY_WRITE_BUFFER,
            src_offset as i32,
            dst_offset as i32,
            size as i32,
        );
        gl.bind_buffer(Gl::COPY_READ_BUFFER, None);
        gl.bind_buffer(Gl::COPY_WRITE_BUFFER, None);
    }

    /// Copies a `width × height` rectangle at (`x`, `y`) of `src`'s color
    /// attachment `attachment` (`FramebufferId::NULL` = the canvas) into level
    /// `level` of 2D texture `tex` at (`x_offset`, `y_offset`)
    /// (`copyTexSubImage2D`). The texture needs storage already, in a format
    /// compatible with the source.
    #[allow(clippy::too_many_arguments)]
    #[wasm_bindgen(js_name = copyTextureSubImage2D)]
    pub fn copy_texture_sub_image_2d(
        &self,
        tex: TextureId,
        level: i32,
        x_offset: i32,
        y_offset: i32,
        src: FramebufferId,
        attachment: u32,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) {
        const OP: &str = "copy_texture_sub_image_2d";
        let r = Region {
            x,
            y,
            width,
            height,
        };
        // An antialiased canvas is resolved for the copy, as for readPixels
        // (WebGL's implicit resolve of the drawing buffer).
        let multisampled = if src.is_null() {
            false
        } else {
            let res = self.res();
            res.framebuffers
                .get(src.0)
                .and_then(|f| f.color_format(&res, attachment))
                .is_some_and(|(_, n)| n > 0)
        };
        if multisampled {
            return console::error(&format!(
                "[luxel] {OP}: the source is multisampled (resolve it with a blit first); \
                 ignored"
            ));
        }
        if let Err(e) = self.bind_read_source(src, attachment, r) {
            return console::error(&format!("[luxel] {OP}: {e}; ignored"));
        }
        self.with_texture(tex, OP, |gl, target, _| {
            if target != Gl::TEXTURE_2D {
                return console::error(&format!("[luxel] {OP}: not a 2D texture; ignored"));
            }
            gl.copy_tex_sub_image_2d(target, level, x_offset, y_offset, x, y, width, height);
        });
    }
}
