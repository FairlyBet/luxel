//! Auto exposure: the frame's brightness measured on the GPU and eased
//! toward, for the display pass (and TAA's weights) to expose by.

use luxel_gl::context::Context;
use luxel_gl::error::Error;
use luxel_gl::ids::{FramebufferId, ParamGroupId, ParamLayoutId, ProgramId, StateId, TextureId};
use luxel_gl::param::{ParamField, ParamLayoutBuilder};
use luxel_gl::shader_source::ShaderSource;
use luxel_gl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use luxel_gl::uniform_buffer::UniformType;

const EXPOSURE_FRAG: &str = include_str!("../shaders/exposure.frag");

/// Auto exposure (ported from Godot 3's GLES3 renderer, see exposure.frag):
/// the frame's brightness shrunk to 1×1 on the GPU, eased toward from last
/// frame's, read by the display (and TAA's weights) as `grey / it`.
pub(crate) struct AutoExposure {
    // (program, layout, group) for the first step, the 3×3 shrinks, the last.
    pub(crate) begin: (ProgramId, ParamLayoutId, ParamGroupId),
    pub(crate) shrink: (ProgramId, ParamLayoutId, ParamGroupId),
    pub(crate) end: (ProgramId, ParamLayoutId, ParamGroupId),
    pub(crate) f_source_size: ParamField,
    pub(crate) f_target_size: ParamField,
    pub(crate) f_adjust: ParamField,
    pub(crate) f_min: ParamField,
    pub(crate) f_max: ParamField,
    pub(crate) f_force: ParamField,
    // 243², 81², 27², 9², 3² (Godot's exposure_shrink but the last).
    pub(crate) chain: Vec<(TextureId, FramebufferId, i32)>,
    // The 1×1 result, this frame's and last's (swapped each frame).
    pub(crate) result: [(TextureId, FramebufferId); 2],
    pub(crate) current: usize,
    // When it last ran (ms; 0: never: the next one sets, not eases).
    pub(crate) last_ms: f64,
    pub(crate) grey: f32,
    pub(crate) min_luminance: f32,
    pub(crate) max_luminance: f32,
    pub(crate) speed: f32,
}

impl AutoExposure {
    pub(crate) const SIZE: i32 = 243;

    pub(crate) fn new(ctx: &Context, vert: &ShaderSource) -> Result<Self, Error> {
        // What is made, freed again if a later step fails.
        let undo = std::cell::RefCell::new(luxel_gl::undo::Undo::new(ctx));
        let program = |define: &str, name: &str, layout: ParamLayoutBuilder| -> Result<_, Error> {
            let frag =
                ShaderSource::plain(name, &format!("#version 300 es\n{define}{EXPOSURE_FRAG}"));
            let p = ctx.create_program_from(vert, &frag)?;
            undo.borrow_mut().push(move |c| c.destroy_program(p));
            let l = ctx.create_param_layout(&layout)?;
            undo.borrow_mut().push(move |c| c.destroy_param_layout(l));
            let g = ctx.create_param_group(l)?;
            undo.borrow_mut().push(move |c| c.destroy_param_group(g));
            Ok((p, l, g))
        };
        let begin = program(
            "#define EXPOSURE_BEGIN\n",
            "exposure.frag (begin)",
            ParamLayoutBuilder::new()
                .texture("u_source")
                .uniform("u_source_size", UniformType::IVec2)
                .uniform("u_target_size", UniformType::IVec2),
        )?;
        let shrink = program(
            "",
            "exposure.frag",
            ParamLayoutBuilder::new().texture("u_source"),
        )?;
        let end = program(
            "#define EXPOSURE_END\n",
            "exposure.frag (end)",
            ParamLayoutBuilder::new()
                .texture("u_source")
                .texture("u_prev")
                .uniform("u_adjust", UniformType::F32)
                .uniform("u_min_luminance", UniformType::F32)
                .uniform("u_max_luminance", UniformType::F32)
                .uniform("u_force", UniformType::F32),
        )?;
        let target = |size: i32| -> Result<(TextureId, FramebufferId), Error> {
            let t = ctx.create_texture(
                TextureTarget::Texture2D,
                TextureMinFilter::Nearest,
                TextureMagFilter::Nearest,
            )?;
            undo.borrow_mut().push(move |c| c.destroy_texture(t));
            ctx.texture_storage_2d(t, 1, &TextureFormat::rgba16f(), size, size);
            let fb = ctx.create_framebuffer(size, size)?;
            undo.borrow_mut().push(move |c| c.destroy_framebuffer(fb));
            ctx.framebuffer_attach_color_texture_2d(fb, 0, t, 0);
            Ok((t, fb))
        };
        let mut chain = Vec::new();
        let mut size = Self::SIZE;
        while size > 1 {
            let (t, fb) = target(size)?;
            chain.push((t, fb, size));
            size /= 3;
        }
        let result = [target(1)?, target(1)?];
        let made = Self {
            f_source_size: ctx.param_field(begin.1, "u_source_size")?,
            f_target_size: ctx.param_field(begin.1, "u_target_size")?,
            f_adjust: ctx.param_field(end.1, "u_adjust")?,
            f_min: ctx.param_field(end.1, "u_min_luminance")?,
            f_max: ctx.param_field(end.1, "u_max_luminance")?,
            f_force: ctx.param_field(end.1, "u_force")?,
            begin,
            shrink,
            end,
            chain,
            result,
            current: 0,
            last_ms: 0.0,
            // Godot's defaults, but the ceiling: physically lit scenes
            // (hundreds of lux) average far above its 8.
            grey: 0.4,
            min_luminance: 0.05,
            max_luminance: 10_000.0,
            speed: 0.5,
        };
        undo.into_inner().keep();
        Ok(made)
    }

    // Shrinks `source` (`w`×`h`) to 1×1 and eases the result (`now_ms`).
    pub(crate) fn meter(
        &mut self,
        ctx: &Context,
        state: StateId,
        source: TextureId,
        w: i32,
        h: i32,
        now_ms: f64,
    ) {
        let draw = |fb: FramebufferId, size: i32, program: ProgramId, group: ParamGroupId| {
            ctx.render(&crate::fullscreen::pass(
                fb,
                (size, size),
                program,
                state,
                group,
            ));
        };
        // First step: the image into the luminance buffer.
        let (p, _, g) = self.begin;
        let (t0, fb0, s0) = self.chain[0];
        ctx.param_set_texture_by_name(g, "u_source", source);
        ctx.param_set_ints(g, self.f_source_size, &[w, h]);
        ctx.param_set_ints(g, self.f_target_size, &[s0, s0]);
        draw(fb0, s0, p, g);
        // Second: shrink down to 3×3.
        let (p, _, g) = self.shrink;
        let mut from = t0;
        for &(t, fb, size) in &self.chain[1..] {
            ctx.param_set_texture_by_name(g, "u_source", from);
            draw(fb, size, p, g);
            from = t;
        }
        // Last: to 1×1, eased from the previous.
        let (adjust, force) = ease(self.last_ms, now_ms, self.speed);
        self.last_ms = now_ms;
        let next = 1 - self.current;
        let (p, _, g) = self.end;
        ctx.param_set_texture_by_name(g, "u_source", from);
        ctx.param_set_texture_by_name(g, "u_prev", self.result[self.current].0);
        ctx.param_set_f32(g, self.f_adjust, adjust);
        ctx.param_set_f32(g, self.f_min, self.min_luminance);
        ctx.param_set_f32(g, self.f_max, self.max_luminance);
        ctx.param_set_f32(g, self.f_force, if force { 1.0 } else { 0.0 });
        draw(self.result[next].1, 1, p, g);
        self.current = next;
    }

    pub(crate) fn texture(&self) -> TextureId {
        self.result[self.current].0
    }

    pub(crate) fn destroy(self, ctx: &Context, lost: bool) {
        for (p, l, g) in [self.begin, self.shrink, self.end] {
            ctx.destroy_param_group(g);
            ctx.destroy_param_layout(l);
            if !lost {
                ctx.destroy_program(p);
            }
        }
        if !lost {
            for (t, fb, _) in self.chain {
                ctx.destroy_framebuffer(fb);
                ctx.destroy_texture(t);
            }
            for (t, fb) in self.result {
                ctx.destroy_framebuffer(fb);
                ctx.destroy_texture(t);
            }
        }
    }
}

// The share of the way to the measured value this meter moves (at most all
// of it), and whether to jump there: on the first meter or after a pause of
// over a second. Two meters in the same millisecond (a coarse clock) or a
// clock going back keep the last value.
fn ease(last_ms: f64, now_ms: f64, speed: f32) -> (f32, bool) {
    let dt = now_ms - last_ms;
    if last_ms <= 0.0 || dt > 1000.0 {
        return (1.0, true);
    }
    if dt.is_nan() || dt <= 0.0 {
        return (0.0, false);
    }
    ((speed * (dt as f32 / 1000.0)).min(1.0), false)
}

#[cfg(test)]
mod tests {
    use super::ease;

    #[test]
    fn easing_steps() {
        assert_eq!(ease(0.0, 5.0, 0.5), (1.0, true), "first meter");
        assert_eq!(ease(100.0, 1200.0, 0.5), (1.0, true), "after a pause");
        assert_eq!(ease(100.0, 100.0, 0.5), (0.0, false), "same millisecond");
        assert_eq!(ease(100.0, 90.0, 0.5), (0.0, false), "clock went back");
        assert_eq!(
            ease(100.0, 600.0, 5.0),
            (1.0, false),
            "never past the target"
        );
        let (a, f) = ease(100.0, 116.0, 0.5);
        assert!(!f && (a - 0.008).abs() < 1e-6);
    }
}
