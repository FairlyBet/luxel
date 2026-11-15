// Helpers the compression tests share: a context on a small canvas, an
// RGBA8 test image and a texture of it with mipmaps.
#![allow(dead_code)]

use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};

use rswebgl::context::{Context, ContextOptions};
use rswebgl::ids::TextureId;
use rswebgl::texture::{TextureFormat, TextureMagFilter, TextureMinFilter, TextureTarget};

pub const SIZE: i32 = 32;

pub const VERT: &str = "#version 300 es
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2)) * 2.0 - 1.0;
    gl_Position = vec4(p, 0.0, 1.0);
}
";

// The texture's level u_level, texel for pixel (raw values: an sRGB
// texture's are encoded back to its stored bytes).
pub const SHOW_FRAG: &str = "#version 300 es
precision highp float;
uniform sampler2D u_tex;
uniform int u_level;
uniform int u_srgb;
out vec4 frag_color;
void main() {
    ivec2 size = textureSize(u_tex, u_level);
    vec4 c = texelFetch(u_tex, min(ivec2(gl_FragCoord.xy), size - 1), u_level);
    if (u_srgb == 1) {
        c.rgb = mix(c.rgb * 12.92, 1.055 * pow(c.rgb, vec3(1.0 / 2.4)) - 0.055,
                    step(0.0031308, c.rgb));
    }
    frag_color = c;
}
";

pub struct Env {
    pub ctx: Context,
    pub gl: Gl,
}

pub fn env() -> Env {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(SIZE as u32);
    canvas.set_height(SIZE as u32);
    let mut options = ContextOptions::new();
    options.antialias = false;
    let ctx = Context::from_canvas(&canvas, &options).expect("ctx");
    // Encoders compile in the background (else they block the page past
    // the runner's timeout).
    ctx.enable_extension(rswebgl::extension::Extension::KhrParallelShaderCompile);
    let gl = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    Env { ctx, gl }
}

// What BC formats hold well: colors along one line (a diagonal gradient
// from one color to another), alpha a ramp of its own.
pub fn image() -> Vec<u8> {
    let mut px = Vec::new();
    for y in 0..SIZE {
        for x in 0..SIZE {
            let t = (x + y) as f32 / (2 * SIZE - 2) as f32;
            let r = (t * 255.0).round() as u8;
            let g = (255.0 - t * 200.0).round() as u8;
            let b = (50.0 + t * 100.0).round() as u8;
            let a = (255 - y * 4).clamp(0, 255) as u8;
            px.extend_from_slice(&[r, g, b, a]);
        }
    }
    px
}

pub fn texture(ctx: &Context, srgb: bool) -> TextureId {
    let format = if srgb {
        TextureFormat::srgb8_alpha8()
    } else {
        TextureFormat::rgba8()
    };
    let t = ctx
        .create_texture(
            TextureTarget::Texture2D,
            TextureMinFilter::LinearMipmapLinear,
            TextureMagFilter::Linear,
        )
        .unwrap();
    ctx.texture_storage_2d(t, 6, &format, SIZE, SIZE);
    ctx.texture_sub_image_2d(t, 0, 0, 0, SIZE, SIZE, &format, &image());
    ctx.texture_generate_mipmaps(t);
    t
}

// Level `level` of `tex`, drawn to the canvas and read back (RGBA bytes,
// the level's size).
