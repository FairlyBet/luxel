//! Direct output without the canvas's depth buffer: with HDR off the scene
//! draws displayable colors straight away, which needs a depth test, so
//! when the canvas was created with `depth: false` (with HDR on it never
//! needs one: everything it draws there is full-screen) it draws into a
//! target of its own — RGBA8 and depth, multisampled with the scene's
//! sample count — and a full-screen copy puts it on the canvas. A draw, not
//! a blit, so a multisampled canvas takes it too. The copy also re-presents
//! the last frame (`Scene::present_frame`).

use rswebgl::context::Context;
use rswebgl::error::Error;
use rswebgl::framebuffer::{ClearMask, InvalidateMask};
use rswebgl::ids::{
    FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, RenderbufferId, StateId, TextureId,
};
use rswebgl::param::ParamLayoutBuilder;
use rswebgl::pass::Pass;
use rswebgl::render_state::RenderState;
use rswebgl::renderbuffer::RenderbufferFormat;
use rswebgl::shader_source::ShaderSource;
use rswebgl::texture::{
    TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget, TextureWrap,
};

// Texel for pixel: the target is the canvas's size.
const COPY_FRAG: &str = "precision highp float;
precision highp sampler2D;
uniform sampler2D u_color;
out vec4 frag_color;
void main() {
    frag_color = texelFetch(u_color, ivec2(gl_FragCoord.xy), 0);
}
";

pub(crate) struct DirectTarget {
    program: ProgramId,
    layout: ParamLayoutId,
    group: ParamGroupId,
    state: StateId,
    generation: u32,
    // Drawn into: multisampled renderbuffers (resolved into `resolve_fb`)
    // or, without samples, `color` itself.
    fb: FramebufferId,
    color_rb: RenderbufferId,
    depth_rb: RenderbufferId,
    resolve_fb: FramebufferId,
    color: TextureId,
    size: (i32, i32),
    samples: i32,
    reverse_z: bool,
}

impl DirectTarget {
    pub fn new(ctx: &Context) -> Result<Self, Error> {
        let prefix = "#version 300 es\n";
        let vert = crate::fullscreen::vertex(prefix);
        let frag = ShaderSource::plain("direct_copy.frag", &format!("{prefix}{COPY_FRAG}"));
        let mut undo = rswebgl::undo::Undo::new(ctx);
        let (program, layout, group) = crate::fullscreen::program(
            ctx,
            &mut undo,
            &vert,
            &frag,
            &ParamLayoutBuilder::new().texture("u_color"),
        )?;
        let mut state = RenderState::new();
        state.depth_test = false;
        state.depth_mask = false;
        undo.keep();
        Ok(Self {
            program,
            layout,
            group,
            state: ctx.render_state(&state),
            generation: ctx.generation(),
            fb: FramebufferId::NULL,
            color_rb: RenderbufferId::NULL,
            depth_rb: RenderbufferId::NULL,
            resolve_fb: FramebufferId::NULL,
            color: TextureId::NULL,
            size: (0, 0),
            samples: 0,
            reverse_z: false,
        })
    }

    /// Whether its GPU objects belong to an earlier context generation.
    pub fn is_lost(&self, ctx: &Context) -> bool {
        self.generation != ctx.generation()
    }

    /// Points `pass` at the target (`w`×`h`, `samples` per pixel), cleared
    /// to the canvas's clear color.
    pub fn prepare(
        &mut self,
        ctx: &Context,
        w: i32,
        h: i32,
        samples: i32,
        pass: &mut Pass,
    ) -> Result<(), Error> {
        let (w, h) = (w.max(1), h.max(1));
        let samples = if samples > 1 {
            samples.min(rswebgl::limits::max_samples() as i32)
        } else {
            0
        };
        let reverse_z = ctx.reverse_z();
        if self.size != (w, h)
            || self.samples != samples
            || self.reverse_z != reverse_z
            || self.fb.is_null()
        {
            self.allocate(ctx, w, h, samples, reverse_z)?;
        }
        let [r, g, b, a] = ctx.default_framebuffer().clear_color_rgba();
        ctx.framebuffer_set_clear_color(self.fb, r, g, b, a);
        pass.set_target(self.fb);
        pass.set_clear(&ClearMask::color_depth());
        // Depth (and the multisampled color, once resolved) isn't read
        // again: tiled GPUs skip writing it back.
        let mut discard = InvalidateMask::none();
        discard.depth = true;
        if self.resolve_fb != self.fb {
            pass.set_resolve(self.resolve_fb);
            discard.color = true;
        } else {
            pass.clear_resolve();
        }
        pass.set_invalidate(&discard);
        Ok(())
    }

    /// Whether it holds a frame drawn at `w`×`h`.
    pub fn holds(&self, w: i32, h: i32) -> bool {
        !self.color.is_null() && self.size == (w.max(1), h.max(1))
    }

    /// Copies the last frame drawn into it onto the canvas.
    pub fn present(&self, ctx: &Context) {
        let (w, h) = self.size;
        ctx.param_set_texture_by_name(self.group, "u_color", self.color);
        ctx.render(&crate::fullscreen::pass(
            FramebufferId::NULL,
            (w, h),
            self.program,
            self.state,
            self.group,
        ));
    }

    fn allocate(
        &mut self,
        ctx: &Context,
        w: i32,
        h: i32,
        samples: i32,
        reverse_z: bool,
    ) -> Result<(), Error> {
        self.free_target(ctx);
        let depth = crate::output::depth_renderbuffer_format(reverse_z);
        let color = ctx.create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )?;
        self.color = color;
        ctx.texture_storage_2d(color, 1, &TextureFormat::rgba8(), w, h);
        ctx.texture_set_wrap_s(color, TextureWrap::ClampToEdge);
        ctx.texture_set_wrap_t(color, TextureWrap::ClampToEdge);
        let made = (|| {
            let fb = ctx.create_framebuffer(w, h)?;
            self.fb = fb;
            if samples > 1 {
                self.color_rb =
                    ctx.create_renderbuffer_multisample(RenderbufferFormat::Rgba8, samples, w, h)?;
                self.depth_rb = ctx.create_renderbuffer_multisample(depth, samples, w, h)?;
                ctx.framebuffer_attach_color_renderbuffer(fb, 0, self.color_rb);
                let resolve = ctx.create_framebuffer(w, h)?;
                self.resolve_fb = resolve;
                ctx.framebuffer_attach_color_texture_2d(resolve, 0, color, 0);
            } else {
                self.depth_rb = ctx.create_renderbuffer(depth, w, h)?;
                ctx.framebuffer_attach_color_texture_2d(fb, 0, color, 0);
                self.resolve_fb = fb;
            }
            ctx.framebuffer_attach_depth_renderbuffer(fb, self.depth_rb);
            if !ctx.framebuffer_is_complete(fb)
                || (self.resolve_fb != fb && !ctx.framebuffer_is_complete(self.resolve_fb))
            {
                return Err(Error::from("the direct target is incomplete"));
            }
            Ok(())
        })();
        if let Err(e) = made {
            self.free_target(ctx);
            return Err(e);
        }
        self.size = (w, h);
        self.samples = samples;
        self.reverse_z = reverse_z;
        Ok(())
    }

    fn free_target(&mut self, ctx: &Context) {
        if self.resolve_fb != self.fb && !self.resolve_fb.is_null() {
            ctx.destroy_framebuffer(self.resolve_fb);
        }
        if !self.fb.is_null() {
            ctx.destroy_framebuffer(self.fb);
        }
        for rb in [self.color_rb, self.depth_rb] {
            if !rb.is_null() {
                ctx.destroy_renderbuffer(rb);
            }
        }
        if !self.color.is_null() {
            ctx.destroy_texture(self.color);
        }
        self.fb = FramebufferId::NULL;
        self.resolve_fb = FramebufferId::NULL;
        self.color_rb = RenderbufferId::NULL;
        self.depth_rb = RenderbufferId::NULL;
        self.color = TextureId::NULL;
        self.size = (0, 0);
    }

    pub fn destroy(mut self, ctx: &Context) {
        ctx.destroy_param_group(self.group);
        ctx.destroy_param_layout(self.layout);
        if self.generation != ctx.generation() {
            return;
        }
        self.free_target(ctx);
        ctx.destroy_program(self.program);
    }
}
