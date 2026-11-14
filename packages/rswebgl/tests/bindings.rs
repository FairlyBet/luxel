// Renderer state-tracking tests against a real WebGL2 context.
// Run with: wasm-pack test --headless --chrome packages/rswebgl
#![cfg(target_arch = "wasm32")]

mod common;

use wasm_bindgen_test::*;

use rswebgl::buffer::BufferUsage;
use rswebgl::context::Context;
use rswebgl::ids::{ParamGroupId, ProgramId, TextureId, UniformBufferId};
use rswebgl::param::ParamLayoutBuilder;
use rswebgl::pass::Batch;
use rswebgl::render_state::RenderState;
use rswebgl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use rswebgl::uniform_buffer::{UboLayout, UniformType};

use common::*;

wasm_bindgen_test_configure!(run_in_browser);

// Each program has a single sampler / block, so each puts it on unit 0 /
// point 0: switching programs puts different objects in the same slot.
const SAMPLER_A_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_a;
out vec4 frag_color;
void main() {
    frag_color = texture(u_a, vec2(0.5));
}
"#;

const SAMPLER_B_FRAG: &str = r#"#version 300 es
precision highp float;
uniform sampler2D u_b;
out vec4 frag_color;
void main() {
    frag_color = texture(u_b, vec2(0.5));
}
"#;

const BLOCK_A_FRAG: &str = r#"#version 300 es
precision highp float;
uniform BlockA { vec4 a_color; };
out vec4 frag_color;
void main() {
    frag_color = a_color;
}
"#;

const BLOCK_B_FRAG: &str = r#"#version 300 es
precision highp float;
uniform BlockB { vec4 b_color; };
out vec4 frag_color;
void main() {
    frag_color = b_color;
}
"#;

fn solid_texture(ctx: &Context, c: [u8; 4]) -> TextureId {
    let t = ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .expect("texture");
    ctx.texture_storage_2d(t, 1, &TextureFormat::rgba8(), 1, 1);
    ctx.texture_sub_image_2d(t, 0, 0, 0, 1, 1, &TextureFormat::rgba8(), &c);
    t
}

fn color_block(ctx: &Context, field: &str, c: [u8; 4]) -> UniformBufferId {
    let layout = UboLayout::new().field(field, UniformType::Vec4);
    let ubo = ctx
        .create_uniform_buffer(&layout, BufferUsage::StaticDraw)
        .expect("ubo");
    let f = |x: u8| x as f32 / 255.0;
    ctx.ubo_set_vec4(ubo, field, f(c[0]), f(c[1]), f(c[2]), f(c[3]));
    ubo
}

// A single-texture / single-block parameter group.
fn texture_group(f: &Fixture, name: &str, tex: TextureId) -> ParamGroupId {
    let layout = f
        .ctx
        .create_param_layout(&ParamLayoutBuilder::new().texture(name))
        .unwrap();
    let g = f.ctx.create_param_group(layout).unwrap();
    f.ctx.param_set_texture_by_name(g, name, tex);
    g
}

fn block_group(f: &Fixture, name: &str, ubo: UniformBufferId) -> ParamGroupId {
    let layout = f
        .ctx
        .create_param_layout(&ParamLayoutBuilder::new().block(name))
        .unwrap();
    let g = f.ctx.create_param_group(layout).unwrap();
    f.ctx.param_set_block_by_name(g, name, ubo);
    g
}

// One batch drawing quad `col` with `program` and `group` in slot 0.
fn quad_batch(f: &Fixture, program: ProgramId, col: i32, group: ParamGroupId) -> Batch {
    let mut b = Batch::new(program, f.ctx.render_state(&RenderState::new()));
    b.set_group(0, group);
    draw_quad(&mut b, f, col);
    b
}

// A → B → A, all on unit 0. Switching back to A must rebind RED even though A's
// own parameters didn't change: the unit's contents are global state.
#[wasm_bindgen_test]
fn texture_unit_rebinds_across_programs() {
    let f = fixture();
    let prog_a = f.ctx.create_program(VERT, SAMPLER_A_FRAG).expect("program");
    let prog_b = f.ctx.create_program(VERT, SAMPLER_B_FRAG).expect("program");
    let a = texture_group(&f, "u_a", solid_texture(&f.ctx, RED));
    let b = texture_group(&f, "u_b", solid_texture(&f.ctx, BLUE));
    render(
        &f,
        vec![
            quad_batch(&f, prog_a, 0, a),
            quad_batch(&f, prog_b, 1, b),
            quad_batch(&f, prog_a, 2, a),
        ],
    );

    assert_eq!(read_quads(&f.gl), [RED, BLUE, RED]);
}

// Same shape for uniform-buffer binding points.
#[wasm_bindgen_test]
fn block_point_rebinds_across_programs() {
    let f = fixture();
    let prog_a = f.ctx.create_program(VERT, BLOCK_A_FRAG).expect("program");
    let prog_b = f.ctx.create_program(VERT, BLOCK_B_FRAG).expect("program");
    let a = block_group(&f, "BlockA", color_block(&f.ctx, "a_color", RED));
    let b = block_group(&f, "BlockB", color_block(&f.ctx, "b_color", BLUE));
    render(
        &f,
        vec![
            quad_batch(&f, prog_a, 0, a),
            quad_batch(&f, prog_b, 1, b),
            quad_batch(&f, prog_a, 2, a),
        ],
    );

    assert_eq!(read_quads(&f.gl), [RED, BLUE, RED]);
}

// Two writes far apart in one block upload as separate runs; both must land.
#[wasm_bindgen_test]
fn scattered_ubo_writes_reach_the_gpu() {
    const FRAG: &str = r#"#version 300 es
precision highp float;
uniform Big { vec4 head; vec4 filler[60]; vec4 tail; };
out vec4 frag_color;
void main() {
    frag_color = head + tail + filler[0] * 0.0;
}
"#;
    let f = fixture();
    let program = f.ctx.create_program(VERT, FRAG).expect("program");
    let layout = UboLayout::new()
        .field("head", UniformType::Vec4)
        .array("filler", UniformType::Vec4, 60)
        .field("tail", UniformType::Vec4);
    let ubo = f
        .ctx
        .create_uniform_buffer(&layout, BufferUsage::DynamicDraw)
        .expect("ubo");
    f.ctx.ubo_set_vec4(ubo, "head", 1.0, 0.0, 0.0, 0.5);
    f.ctx.ubo_set_vec4(ubo, "tail", 0.0, 0.0, 1.0, 0.5);

    let g = block_group(&f, "Big", ubo);
    render(&f, vec![quad_batch(&f, program, 1, g)]);

    assert_eq!(read_quads(&f.gl), [BLACK, [255, 0, 255, 255], BLACK]);
}

// A group's block repointed from one buffer to another (as after a context
// loss, the app's buffer recreated) draws with the new one.
#[wasm_bindgen_test]
fn remapped_blocks_draw_with_the_new_buffer() {
    let f = fixture();
    let prog = f.ctx.create_program(VERT, BLOCK_A_FRAG).expect("program");
    let old = color_block(&f.ctx, "a_color", RED);
    let new = color_block(&f.ctx, "a_color", GREEN);
    let g = block_group(&f, "BlockA", old);
    f.ctx
        .remap_group_blocks(&std::collections::HashMap::from([(old, new)]));
    render(&f, vec![quad_batch(&f, prog, 1, g)]);

    assert_eq!(read_quads(&f.gl), [BLACK, GREEN, BLACK]);
}

// An upload binds its texture on the active unit behind the binding cache's
// back (and unbinds it after, from its own target only). A draw that then
// leaves that unit's texture field unset must sample nothing — not the 2D
// texture an earlier draw left there (the cache had taken the unit for
// empty after the upload, and skipped the unbind).
#[wasm_bindgen_test]
fn an_unset_texture_samples_nothing_after_an_upload() {
    let f = fixture();
    let prog = f.ctx.create_program(VERT, SAMPLER_A_FRAG).expect("program");
    let red = texture_group(&f, "u_a", solid_texture(&f.ctx, RED));
    let unset = texture_group(&f, "u_a", TextureId::NULL);
    render(&f, vec![quad_batch(&f, prog, 0, red)]);
    // A 2D-array texture made and filled on unit 0, where RED is bound.
    let array = f
        .ctx
        .create_texture(
            TextureTarget::Texture2DArray,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .expect("texture");
    f.ctx
        .texture_storage_3d(array, 1, &TextureFormat::rgba8(), 1, 1, 1);
    render(&f, vec![quad_batch(&f, prog, 1, unset)]);

    assert_eq!(read_quads(&f.gl)[1], BLACK);
}

// A unit holds one texture: binding one of another target (a 2D array after
// a 2D texture) takes the old one off its target.
#[wasm_bindgen_test]
fn a_unit_holds_one_texture_across_targets() {
    const ARRAY_FRAG: &str = r#"#version 300 es
precision highp float;
precision highp sampler2DArray;
uniform sampler2DArray u_b;
out vec4 frag_color;
void main() {
    frag_color = texture(u_b, vec3(0.5, 0.5, 0.0));
}
"#;
    let f = fixture();
    let prog_2d = f.ctx.create_program(VERT, SAMPLER_A_FRAG).expect("program");
    let prog_array = f.ctx.create_program(VERT, ARRAY_FRAG).expect("program");
    let array = f
        .ctx
        .create_texture(
            TextureTarget::Texture2DArray,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .expect("texture");
    f.ctx
        .texture_storage_3d(array, 1, &TextureFormat::rgba8(), 1, 1, 1);
    f.ctx
        .texture_sub_image_3d(array, 0, 0, 0, 0, 1, 1, 1, &TextureFormat::rgba8(), &BLUE);
    let a = texture_group(&f, "u_a", solid_texture(&f.ctx, RED));
    let b = texture_group(&f, "u_b", array);
    render(
        &f,
        vec![
            quad_batch(&f, prog_2d, 0, a),
            quad_batch(&f, prog_array, 1, b),
        ],
    );

    assert_eq!(read_quads(&f.gl)[..2], [RED, BLUE]);
    f.gl.active_texture(web_sys::WebGl2RenderingContext::TEXTURE0);
    let bound_2d =
        f.gl.get_parameter(web_sys::WebGl2RenderingContext::TEXTURE_BINDING_2D)
            .unwrap();
    assert!(bound_2d.is_null(), "unit 0 still holds the 2D texture");
}
