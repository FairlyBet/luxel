// Glass shows the transparent (alpha-blended) meshes behind it: they are
// drawn into the scene copy it refracts too, as Khronos' renderer does. A
// red blended quad behind a clear transmissive one: red at the center, in
// the fast mode and peeled. Run with:
// wasm-pack test --headless --chrome packages/luxel
#![cfg(target_arch = "wasm32")]

use base64::Engine;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};

use luxel::Scene;
use luxel_gl::context::{Context, ContextOptions};
use luxel_gltf::load_model;

const SIZE: i32 = 32;

// Two 1×1 quads: a red alpha-blended one at z = 0 behind a clear glass one
// at z = 1 (a smaller one: the glass covers the center only).
fn asset_url() -> String {
    let mut bin = Vec::new();
    #[rustfmt::skip]
    let floats = [-0.5f32, -0.5, 0.0, 0.5, -0.5, 0.0, 0.5, 0.5, 0.0, -0.5, 0.5, 0.0];
    for v in floats {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    for i in [0u16, 1, 2, 0, 2, 3] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "extensionsUsed": ["KHR_materials_transmission", "KHR_materials_unlit"],
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
    {{ "name": "Behind", "alphaMode": "BLEND", "pbrMetallicRoughness": {{ "baseColorFactor": [1, 0, 0, 1] }},
       "extensions": {{ "KHR_materials_unlit": {{}} }} }},
    {{ "name": "Glass", "pbrMetallicRoughness": {{ "baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 0 }},
       "extensions": {{ "KHR_materials_transmission": {{ "transmissionFactor": 1 }} }} }}
  ],
  "meshes": [
    {{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 0 }}] }},
    {{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 1 }}] }}
  ],
  "nodes": [
    {{ "mesh": 0, "scale": [2, 2, 1] }},
    {{ "mesh": 1, "translation": [0, 0, 1] }}
  ],
  "scenes": [{{ "nodes": [0, 1] }}],
  "scene": 0
}}"#,
        len = bin.len(),
        bin = b64(&bin),
    );
    format!("data:model/gltf+json;base64,{}", b64(json.as_bytes()))
}

#[wasm_bindgen_test]
async fn glass_shows_transparent_meshes_behind_it() {
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
    ctx.default_framebuffer()
        .set_clear_color(0.0, 0.0, 0.0, 1.0);
    let gl: Gl = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let mut scene = Scene::new(&ctx).expect("scene");
    if !scene.hdr() {
        return; // glass sees a scene copy only with the HDR target
    }
    scene.set_tone_mapping("none").unwrap();
    scene.set_dither(false);
    scene.set_samples(0);
    scene.set_environment_intensity(0.0, 0.0, 0.0);
    let camera = scene.spawn();
    let cam =
        js_sys::JSON::parse(r#"{ "type": "orthographic", "ymag": 1, "znear": 0.1, "zfar": 100 }"#)
            .unwrap();
    scene.set_camera(camera, cam).unwrap();
    scene.look_at(camera, &[0.0, 0.0, 5.0], &[0.0, 0.0, 0.0]);
    let root = scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    for layers in [0u32, 2] {
        scene.set_transmission_layers(layers);
        wasm_bindgen_futures::JsFuture::from(scene.when_ready())
            .await
            .unwrap();
        scene.render_frame(camera);
        scene.render_frame(camera);
        let mut px = [0u8; 4];
        gl.read_pixels_with_opt_u8_array(
            SIZE / 2,
            SIZE / 2,
            1,
            1,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            Some(&mut px),
        )
        .unwrap();
        assert!(px[0] > 150 && px[1] < 60, "layers {layers}: center {px:?}");
    }
    // Nothing behind the glass but a background color: the glass shows it
    // (else the empty, black scene copy).
    let behind = scene
        .children(root)
        .into_iter()
        .map(luxel_ecs::Entity::from_raw)
        .find(|&e| scene.local_matrix(e)[0] == 2.0)
        .expect("the red quad's node");
    scene.set_visible(behind, false);
    scene.set_background_color(&[0.0, 1.0, 0.0]).unwrap();
    let px = center(&gl, &mut scene, camera).await;
    assert!(px[1] > 150 && px[0] < 60, "glass over green: {px:?}");
    // Straight to the canvas (no scene copy): the fill still, not the
    // (black) environment.
    scene.set_hdr(false);
    let px = center(&gl, &mut scene, camera).await;
    assert!(
        px[1] > 150 && px[0] < 60,
        "direct: glass over green: {px:?}"
    );
    scene.destroy();
}

// OpenPBR's transmission_color with transmission_depth 0 (no medium) tints
// the light crossing the surface: white behind clear glass shows green with
// [0, 1, 0]; glTF ref (glTF has no such tint) keeps it white.
#[wasm_bindgen_test]
async fn openpbr_transmission_color_without_depth_tints_the_surface() {
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
    let gl: Gl = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let mut scene = Scene::new(&ctx).expect("scene");
    if !scene.hdr() {
        return;
    }
    scene.set_tone_mapping("none").unwrap();
    scene.set_dither(false);
    scene.set_samples(0);
    scene.set_environment_intensity(0.0, 0.0, 0.0);
    let camera = scene.spawn();
    let cam =
        js_sys::JSON::parse(r#"{ "type": "orthographic", "ymag": 1, "znear": 0.1, "zfar": 100 }"#)
            .unwrap();
    scene.set_camera(camera, cam).unwrap();
    scene.look_at(camera, &[0.0, 0.0, 5.0], &[0.0, 0.0, 0.0]);
    let root = scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    let json = |v: &str| js_sys::JSON::parse(v).unwrap();
    let behind = scene.find_material(root, "Behind");
    scene
        .set_material_property(behind, "baseColorFactor", json("[1, 1, 1, 1]"))
        .unwrap();
    let glass = scene.find_material(root, "Glass");
    scene
        .set_material_property(glass, "transmission_color", json("[0, 1, 0]"))
        .unwrap();
    scene.set_shading_model("openpbr").unwrap();
    let tinted = center(&gl, &mut scene, camera).await;
    assert!(tinted[1] > 150 && tinted[0] < 60, "openpbr: {tinted:?}");
    scene.set_shading_model("gltf").unwrap();
    let plain = center(&gl, &mut scene, camera).await;
    assert!(plain[0] > 150 && plain[1] > 150, "gltf: {plain:?}");
    scene.destroy();
}

// A loaded material means what glTF means: its base color tints the light
// through it, under the OpenPBR model too. Marked OpenPBR-native, only
// transmission_color does: green glass in front of white shows white.
#[wasm_bindgen_test]
async fn openpbr_native_glass_is_not_tinted_by_its_base_color() {
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
    let gl: Gl = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let mut scene = Scene::new(&ctx).expect("scene");
    if !scene.hdr() {
        return;
    }
    scene.set_tone_mapping("none").unwrap();
    scene.set_dither(false);
    scene.set_samples(0);
    scene.set_environment_intensity(0.0, 0.0, 0.0);
    let camera = scene.spawn();
    let cam =
        js_sys::JSON::parse(r#"{ "type": "orthographic", "ymag": 1, "znear": 0.1, "zfar": 100 }"#)
            .unwrap();
    scene.set_camera(camera, cam).unwrap();
    scene.look_at(camera, &[0.0, 0.0, 5.0], &[0.0, 0.0, 0.0]);
    let root = scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    let json = |v: &str| js_sys::JSON::parse(v).unwrap();
    let behind = scene.find_material(root, "Behind");
    scene
        .set_material_property(behind, "baseColorFactor", json("[1, 1, 1, 1]"))
        .unwrap();
    let glass = scene.find_material(root, "Glass");
    scene
        .set_material_property(glass, "base_color", json("[0, 1, 0]"))
        .unwrap();
    scene.set_shading_model("openpbr").unwrap();
    assert_eq!(scene.material_semantics(glass).as_deref(), Some("gltf"));
    let gltf = center(&gl, &mut scene, camera).await;
    assert!(gltf[1] > 150 && gltf[0] < 60, "gltf data: {gltf:?}");
    scene.set_material_semantics(glass, "openpbr").unwrap();
    let native = center(&gl, &mut scene, camera).await;
    assert!(
        native[0] > 150 && native[1] > 150,
        "openpbr native: {native:?}"
    );
    assert!(scene.set_material_semantics(glass, "mtlx").is_err());
    scene.destroy();
}

// The center pixel after two frames.
async fn center(gl: &Gl, scene: &mut Scene, camera: luxel_ecs::Entity) -> [u8; 4] {
    wasm_bindgen_futures::JsFuture::from(scene.when_ready())
        .await
        .unwrap();
    scene.render_frame(camera);
    scene.render_frame(camera);
    let mut px = [0u8; 4];
    gl.read_pixels_with_opt_u8_array(
        SIZE / 2,
        SIZE / 2,
        1,
        1,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(&mut px),
    )
    .unwrap();
    px
}
