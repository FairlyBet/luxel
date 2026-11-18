// A canvas without depth (`depth: false`): with HDR off the scene draws into
// a target of its own, so the depth test still holds; `presentFrame` shows
// the last frame again, exactly, through HDR or not, and declines once the
// canvas resized. Run with:
// wasm-pack test --headless --chrome packages/luxel
#![cfg(target_arch = "wasm32")]

use base64::Engine;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};

use luxel::Scene;
use luxel_gl::context::{Context, ContextOptions};
use luxel_gl::draw::Viewport;
use luxel_gltf::load_model;

const SIZE: i32 = 16;

// Two unlit quads facing +Z over the view's center: green at z = 1 (near),
// red and twice as large at z = 0 (far).
fn asset_url() -> String {
    let mut bin = Vec::new();
    for v in [
        -0.5f32, -0.5, 0.0, 0.5, -0.5, 0.0, 0.5, 0.5, 0.0, -0.5, 0.5, 0.0,
    ] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "extensionsUsed": ["KHR_materials_unlit"],
  "buffers": [{{ "byteLength": {len}, "uri": "data:application/octet-stream;base64,{bin}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 48 }},
    {{ "buffer": 0, "byteOffset": 48, "byteLength": 12, "target": 34963 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
       "min": [-0.5, -0.5, 0], "max": [0.5, 0.5, 0] }},
    {{ "bufferView": 1, "componentType": 5123, "count": 6, "type": "SCALAR" }}
  ],
  "materials": [
    {{ "pbrMetallicRoughness": {{ "baseColorFactor": [0, 1, 0, 1] }}, "extensions": {{ "KHR_materials_unlit": {{}} }} }},
    {{ "pbrMetallicRoughness": {{ "baseColorFactor": [1, 0, 0, 1] }}, "extensions": {{ "KHR_materials_unlit": {{}} }} }}
  ],
  "meshes": [
    {{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 0 }}] }},
    {{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 1 }}] }}
  ],
  "nodes": [
    {{ "mesh": 0, "translation": [0, 0, 1] }},
    {{ "mesh": 1, "scale": [2, 2, 1] }}
  ],
  "scenes": [{{ "nodes": [0, 1] }}],
  "scene": 0
}}"#,
        len = bin.len(),
        bin = b64(&bin),
    );
    format!("data:model/gltf+json;base64,{}", b64(json.as_bytes()))
}

fn read(gl: &Gl) -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    gl.read_pixels_with_opt_u8_array(0, 0, SIZE, SIZE, Gl::RGBA, Gl::UNSIGNED_BYTE, Some(&mut px))
        .unwrap();
    px
}

fn center(px: &[u8]) -> [u8; 3] {
    let i = ((SIZE / 2 * SIZE + SIZE / 2) * 4) as usize;
    [px[i], px[i + 1], px[i + 2]]
}

#[wasm_bindgen_test]
async fn a_canvas_without_depth_still_depth_tests_and_presents_again() {
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
    options.depth = false;
    let ctx = Context::from_canvas(&canvas, &options).expect("ctx");
    let gl: Gl = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    assert!(!ctx.default_framebuffer().has_depth());
    let mut scene = Scene::new(&ctx).expect("scene");
    scene.set_tone_mapping("none").unwrap();
    scene.set_dither(false);
    scene.set_samples(0);
    let camera = scene.spawn();
    let cam =
        js_sys::JSON::parse(r#"{ "type": "orthographic", "ymag": 1, "znear": 0.1, "zfar": 100 }"#)
            .unwrap();
    scene.set_camera(camera, cam).unwrap();
    scene.look_at(camera, &[0.0, 0.0, 5.0], &[0.0, 0.0, 0.0]);
    scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    wasm_bindgen_futures::JsFuture::from(scene.when_ready())
        .await
        .unwrap();

    assert!(!scene.present_frame(), "nothing drawn yet");
    for hdr in [false, true] {
        if scene.set_hdr(hdr) != hdr {
            continue; // no HDR output on this device
        }
        scene.render_frame(camera);
        let drawn = read(&gl);
        let [r, g, _] = center(&drawn);
        assert!(g > 200 && r < 50, "hdr {hdr}: the near quad wins: {r} {g}");
        assert!(scene.present_frame(), "hdr {hdr}: presents again");
        assert!(read(&gl) == drawn, "hdr {hdr}: the same picture");
    }

    // Resized: the frame held is the old size.
    canvas.set_width(SIZE as u32 * 2);
    ctx.default_framebuffer()
        .set_viewport(Viewport::new(0, 0, SIZE * 2, SIZE));
    assert!(!scene.present_frame(), "declines after a resize");
    scene.destroy();
}
