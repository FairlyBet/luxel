//! The fullscreen triangle every post-pass draws: three vertices made from
//! `gl_VertexID` (`shaders/fullscreen.vert`), no vertex array.

use rswebgl::draw::{DrawCommand, DrawMode, Viewport};
use rswebgl::ids::{FramebufferId, ParamGroupId, ProgramId, StateId, VertexArrayId};
use rswebgl::param::GROUP_SLOTS;
use rswebgl::pass::Pass;
use rswebgl::shader_source::ShaderSource;

/// The vertex shader's body (without a `#version` line).
pub(crate) const FULLSCREEN_VERT: &str = include_str!("../shaders/fullscreen.vert");

/// The vertex shader after `prefix` (its `#version` line and defines).
pub(crate) fn vertex(prefix: &str) -> ShaderSource {
    ShaderSource::plain("fullscreen.vert", &format!("{prefix}{FULLSCREEN_VERT}"))
}

/// A pass into `target` (NULL: the canvas) over `w`×`h`, drawing the
/// triangle once with `program`, `state` and `group` in slot 0. Set its
/// clear or time it, then render it.
pub(crate) fn pass(
    target: FramebufferId,
    (w, h): (i32, i32),
    program: ProgramId,
    state: StateId,
    group: ParamGroupId,
) -> Pass {
    let mut pass = Pass::new(target);
    pass.set_viewport(&Viewport::new(0, 0, w, h));
    let mut groups = [ParamGroupId::NULL; GROUP_SLOTS];
    groups[0] = group;
    pass.push_groups_with_key(
        0,
        program,
        state,
        VertexArrayId::NULL,
        groups,
        &DrawCommand::arrays(DrawMode::Triangles, 0, 3),
    );
    pass
}

/// A post-pass's program (`vert` + `frag`), its layout and one group, each
/// registered in `undo` so a constructor failing later frees them.
pub(crate) fn program(
    ctx: &rswebgl::context::Context,
    undo: &mut rswebgl::undo::Undo,
    vert: &ShaderSource,
    frag: &ShaderSource,
    layout: &rswebgl::param::ParamLayoutBuilder,
) -> Result<(ProgramId, rswebgl::ids::ParamLayoutId, ParamGroupId), rswebgl::error::Error> {
    let program = ctx.create_program_from(vert, frag)?;
    undo.push(move |c| c.destroy_program(program));
    let layout = ctx.create_param_layout(layout)?;
    undo.push(move |c| c.destroy_param_layout(layout));
    let group = ctx.create_param_group(layout)?;
    undo.push(move |c| c.destroy_param_group(group));
    Ok((program, layout, group))
}
