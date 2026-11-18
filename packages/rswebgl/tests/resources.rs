// Resource handles: explicit destroy, stale-handle detection, deferred destroy
// of resources still in use. Run with:
// wasm-pack test --headless --chrome packages/rswebgl
#![cfg(target_arch = "wasm32")]

use crate::common;

use wasm_bindgen_test::*;
use web_sys::WebGl2RenderingContext;

use rswebgl::buffer::BufferUsage;
use rswebgl::draw::{DrawCommand, DrawMode, IndexType};
use rswebgl::ids::{ProgramId, TextureId};
use rswebgl::pass::Batch;
use rswebgl::render_state::RenderState;
use rswebgl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};
use rswebgl::vao::VertexAttr;

use common::*;

const RED_FRAG: &str = r#"#version 300 es
precision highp float;
out vec4 frag_color;
void main() {
    frag_color = vec4(1.0, 0.0, 0.0, 1.0);
}
"#;

fn cmd() -> DrawCommand {
    DrawCommand::elements(DrawMode::Triangles, 6, IndexType::UnsignedShort, 0)
}

fn red_batch(f: &Fixture, program: ProgramId, col: i32) -> Batch {
    let mut b = Batch::new(program, f.ctx.render_state(&RenderState::new()));
    draw_quad(&mut b, f, col);
    b
}

fn new_texture(f: &Fixture) -> TextureId {
    let t = f
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .unwrap();
    f.ctx
        .texture_storage_2d(t, 1, &TextureFormat::rgba8(), 4, 4);
    t
}

// A destroyed handle is detected: queries return their "nothing" value, and
// operations report once instead of reaching GL.
#[wasm_bindgen_test]
fn destroyed_handle_is_stale() {
    let f = fixture();
    let buf = f
        .ctx
        .create_buffer(BufferUsage::StaticDraw, &[0; 64])
        .unwrap();
    assert_eq!(f.ctx.buffer_size(buf), 64);
    f.ctx.destroy_buffer(buf);
    assert_eq!(f.ctx.buffer_size(buf), 0);

    let errors = capture_console("error", || {
        f.ctx.buffer_write(buf, 0, &[1, 2, 3, 4]);
        f.ctx.destroy_buffer(buf);
    });
    assert_eq!(errors.len(), 2, "{errors:?}");
    assert!(errors.iter().all(|e| e.contains("destroyed or invalid")));
    assert_eq!(f.gl.get_error(), WebGl2RenderingContext::NO_ERROR);

    // A new object reusing the slot is not reachable through the old handle.
    let buf2 = f
        .ctx
        .create_buffer(BufferUsage::StaticDraw, &[0; 8])
        .unwrap();
    assert_ne!(buf2, buf);
    assert_eq!(f.ctx.buffer_size(buf), 0);
    assert_eq!(f.ctx.buffer_size(buf2), 8);
}

// Handles are plain numbers on the JS side.
#[wasm_bindgen_test]
fn handles_are_numbers() {
    let f = fixture();
    let t = new_texture(&f);
    let raw = t.to_raw();
    assert_ne!(raw, 0);
    assert_eq!(TextureId::from_raw(raw), t);
}

// Destroying a buffer a vertex array still reads defers the GL delete: the
// vertex array keeps drawing until it's destroyed itself.
#[wasm_bindgen_test]
fn buffer_in_use_by_vertex_array_survives_until_released() {
    let f = fixture();
    let program = f.ctx.create_program(VERT, RED_FRAG).unwrap();
    let verts: Vec<u8> = [0.0f32, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let idx: Vec<u8> = [0u16, 1, 2, 0, 2, 3]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    let vbo = f
        .ctx
        .create_buffer(BufferUsage::StaticDraw, &verts)
        .unwrap();
    let ibo = f
        .ctx
        .create_index_buffer(BufferUsage::StaticDraw, &idx)
        .unwrap();
    let vao = f.ctx.create_vertex_array().unwrap();
    f.ctx.vertex_array_attr(vao, 0, vbo, &VertexAttr::float(2));
    f.ctx.vertex_array_set_index_buffer(vao, ibo);

    f.ctx.destroy_buffer(vbo);
    f.ctx.destroy_buffer(ibo);
    // The handles are stale right away…
    assert_eq!(f.ctx.buffer_size(vbo), 0);
    // …but the GL buffers live on while the vertex array uses them.
    let mut b = Batch::new(program, f.ctx.render_state(&RenderState::new()));
    b.draw_with_group(vao, QUAD_SLOT, quad(&f, 1), &cmd());
    render(&f, vec![b]);
    assert_eq!(read_quads(&f.gl), [BLACK, RED, BLACK]);

    // Destroying the vertex array releases (and now deletes) them.
    f.ctx.destroy_vertex_array(vao);
    assert_eq!(f.gl.get_error(), WebGl2RenderingContext::NO_ERROR);
}

// Same for a texture attached to a framebuffer.
#[wasm_bindgen_test]
fn texture_attached_to_framebuffer_survives_until_detached() {
    let f = fixture();
    let tex = new_texture(&f);
    let fb = f.ctx.create_framebuffer(4, 4).unwrap();
    f.ctx.framebuffer_attach_color_texture_2d(fb, 0, tex, 0);
    f.ctx.destroy_texture(tex);
    assert!(f.ctx.texture_target(tex).is_none());
    // The framebuffer still has a complete attachment.
    assert!(f.ctx.framebuffer_is_complete(fb));
    f.ctx.destroy_framebuffer(fb);
    assert_eq!(f.gl.get_error(), WebGl2RenderingContext::NO_ERROR);
}

// A program destroyed after a pass was built: that batch is skipped (reported
// once), the rest of the pass still draws.
#[wasm_bindgen_test]
fn destroyed_program_in_a_built_pass_is_skipped() {
    let f = fixture();
    let gone = f.ctx.create_program(VERT, RED_FRAG).unwrap();
    let kept = f.ctx.create_program(VERT, RED_FRAG).unwrap();
    let batches = vec![red_batch(&f, gone, 0), red_batch(&f, kept, 2)];
    f.ctx.destroy_program(gone);

    let errors = capture_console("error", || render(&f, batches));
    assert!(
        errors.iter().any(|e| e.contains("destroyed or invalid")),
        "{errors:?}"
    );
    assert_eq!(read_quads(&f.gl), [BLACK, BLACK, RED]);
}

// Mipmap generation is refused, with a reason, for formats GL can't generate
// them for — instead of a GL error and a texture that samples black.
#[wasm_bindgen_test]
fn mipmaps_only_for_formats_that_can_have_them() {
    let f = fixture();
    let make = |format: &TextureFormat| {
        let t = f
            .ctx
            .create_texture(
                TextureTarget::Texture2D,
                TextureMinFilter::Nearest,
                TextureMagFilter::Nearest,
            )
            .unwrap();
        f.ctx.texture_storage_2d(t, 2, format, 2, 2);
        t
    };
    let generate = |t: TextureId| {
        let errors = capture_console("error", || f.ctx.texture_generate_mipmaps(t));
        (errors, f.gl.get_error())
    };
    while f.gl.get_error() != WebGl2RenderingContext::NO_ERROR {}

    let (errors, gl_error) = generate(make(&TextureFormat::rgba8()));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(gl_error, WebGl2RenderingContext::NO_ERROR);

    let rgba8ui = TextureFormat {
        internal: 0x8D7C, // RGBA8UI (i32)
        format: 0x8D99,   // RGBA_INTEGER
        data_type: WebGl2RenderingContext::UNSIGNED_BYTE,
    };
    let (errors, gl_error) = generate(make(&rgba8ui));
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].contains("can't have mipmaps"), "{errors:?}");
    assert_eq!(gl_error, WebGl2RenderingContext::NO_ERROR);

    let unallocated = f
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .unwrap();
    let (errors, gl_error) = generate(unallocated);
    assert!(errors[0].contains("no storage"), "{errors:?}");
    assert_eq!(gl_error, WebGl2RenderingContext::NO_ERROR);
}

// A 2D array's layers don't bound its mip levels (a 3D texture's depth
// does): a 64×64 array of 512 layers takes 7 levels, not 10.
#[wasm_bindgen_test]
fn array_mip_levels_follow_width_and_height_only() {
    let f = fixture();
    let make = |levels: i32| {
        let t = f
            .ctx
            .create_texture(
                TextureTarget::Texture2DArray,
                TextureMinFilter::Nearest,
                TextureMagFilter::Nearest,
            )
            .unwrap();
        let errors = capture_console("error", || {
            f.ctx
                .texture_storage_3d(t, levels, &TextureFormat::rgba8(), 64, 64, 512)
        });
        (t, errors)
    };
    let (ok, errors) = make(7);
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(f.gl.get_error(), WebGl2RenderingContext::NO_ERROR);
    let (_, errors) = make(10);
    assert!(
        errors.iter().any(|e| e.contains("mip levels")),
        "{errors:?}"
    );
    assert_eq!(
        f.gl.get_error(),
        WebGl2RenderingContext::NO_ERROR,
        "rejected before GL"
    );
    f.ctx.destroy_texture(ok);
}

// Live objects and their memory, counted up and back down.
#[wasm_bindgen_test]
fn resource_stats_count_objects_and_bytes() {
    use rswebgl::renderbuffer::RenderbufferFormat;
    let f = fixture();
    let stat = |k: &str| {
        js_sys::Reflect::get(&f.ctx.resource_stats(), &k.into())
            .unwrap()
            .as_f64()
            .unwrap()
    };
    let (buffers, bytes) = (stat("buffers"), stat("bufferBytes"));
    let (textures, tex_bytes, rb_bytes) = (
        stat("textures"),
        stat("textureBytes"),
        stat("renderbufferBytes"),
    );
    let buf = f
        .ctx
        .create_buffer(BufferUsage::StaticDraw, &[0; 100])
        .unwrap();
    let tex = f
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Linear,
            TextureMagFilter::Linear,
        )
        .unwrap();
    // 64² RGBA8 with a full chain: 64² + 32² + … + 1 texels × 4 bytes.
    f.ctx
        .texture_storage_2d(tex, 7, &TextureFormat::rgba8(), 64, 64);
    let rb = f
        .ctx
        .create_renderbuffer_multisample(RenderbufferFormat::Rgba8, 4, 8, 8)
        .unwrap();
    assert_eq!(stat("buffers"), buffers + 1.0);
    assert_eq!(stat("bufferBytes"), bytes + 100.0);
    assert_eq!(stat("textures"), textures + 1.0);
    let chain: f64 = (0..7).map(|l| ((64 >> l) * (64 >> l) * 4) as f64).sum();
    assert_eq!(stat("textureBytes"), tex_bytes + chain);
    assert_eq!(stat("renderbufferBytes"), rb_bytes + (8 * 8 * 4 * 4) as f64);
    f.ctx.destroy_buffer(buf);
    f.ctx.destroy_texture(tex);
    f.ctx.destroy_renderbuffer(rb);
    assert_eq!(stat("buffers"), buffers);
    assert_eq!(stat("textureBytes"), tex_bytes);
    assert_eq!(stat("renderbufferBytes"), rb_bytes);
}

// Raw GL interop: the objects behind handles, and the state cache reset
// after code of one's own changed GL state.
#[wasm_bindgen_test]
fn raw_gl_and_a_state_reset() {
    let f = fixture();
    let frag = "#version 300 es\nprecision highp float;\nout vec4 c;\n\
                void main() { c = vec4(1.0, 0.0, 0.0, 1.0); }\n";
    let program = f.ctx.create_program(VERT, frag).unwrap();
    let state = f.ctx.render_state(&RenderState::new());
    let frame = || {
        let mut b = Batch::new(program, state);
        draw_quad(&mut b, &f, 1);
        render(&f, vec![b]);
        read_quads(&f.gl)[1]
    };
    assert_eq!(frame(), RED);
    // Foreign GL: another program current, no vertex array, a blend that
    // zeroes everything, a color mask off.
    let gl = f.ctx.raw_gl();
    let other = f.ctx.create_program(VERT, frag).unwrap();
    gl.use_program(f.ctx.raw_program(other).as_ref());
    gl.bind_vertex_array(None);
    gl.enable(WebGl2RenderingContext::BLEND);
    gl.blend_func(WebGl2RenderingContext::ZERO, WebGl2RenderingContext::ZERO);
    gl.color_mask(false, false, false, false);
    f.ctx.reset_gl_state();
    assert_eq!(frame(), RED);
    // The raw objects are the handles' own.
    let tex = f
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Linear,
            TextureMagFilter::Linear,
        )
        .unwrap();
    assert!(gl.is_texture(f.ctx.raw_texture(tex).as_ref()));
    f.ctx.destroy_texture(tex);
    assert!(f.ctx.raw_texture(tex).is_none());
}

// Dropping the last handle of a context deletes its GL objects at once, not
// when the canvas is garbage-collected: an app that rebuilds its viewer
// mustn't keep the old one's GPU memory until then.
#[wasm_bindgen_test]
fn dropping_the_context_deletes_its_objects() {
    let f = fixture();
    let tex = f
        .ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::Nearest,
            TextureMagFilter::Nearest,
        )
        .unwrap();
    f.ctx
        .texture_storage_2d(tex, 1, &TextureFormat::rgba8(), 4, 4);
    let buf = f
        .ctx
        .create_buffer(BufferUsage::StaticDraw, &[0; 64])
        .unwrap();
    let program = f.ctx.create_program(VERT, RED_FRAG).unwrap();
    let raw_tex = f.ctx.raw_texture(tex).unwrap();
    let raw_buf = f.ctx.raw_buffer(buf).unwrap();
    let raw_program = f.ctx.raw_program(program).unwrap();
    assert!(f.gl.is_texture(Some(&raw_tex)) && f.gl.is_buffer(Some(&raw_buf)));
    assert!(f.gl.is_program(Some(&raw_program)));

    let Fixture { ctx, gl, .. } = f;
    drop(ctx);
    assert!(!gl.is_texture(Some(&raw_tex)), "texture deleted");
    assert!(!gl.is_buffer(Some(&raw_buf)), "buffer deleted");
    assert!(!gl.is_program(Some(&raw_program)), "program deleted");
    assert_eq!(gl.get_error(), WebGl2RenderingContext::NO_ERROR);
}
