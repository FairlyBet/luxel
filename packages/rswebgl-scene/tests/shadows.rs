// Shadow maps: a small square above a floor, lit at a slant by a
// directional light, darkens the floor where its shadow falls — and only
// with castShadow. Run with:
// wasm-pack test --headless --chrome packages/rswebgl-scene
#![cfg(target_arch = "wasm32")]

use base64::Engine;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{HtmlCanvasElement, WebGl2RenderingContext as Gl};

use rswebgl::context::{Context, ContextOptions};
use rswebgl_gltf::load_model;
use rswebgl_scene::Scene;

wasm_bindgen_test_configure!(run_in_browser);

const SIZE: i32 = 32;

// A 2×2 floor at z = 0 and a 0.4×0.4 square at z = 1 over its center, both
// facing +Z (white, rough, dielectric).
fn asset_url() -> String {
    let mut bin = Vec::new();
    #[rustfmt::skip]
    let floats = [
        -0.5f32, -0.5, 0.0, 0.5, -0.5, 0.0, 0.5, 0.5, 0.0, -0.5, 0.5, 0.0,
        0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0,
    ];
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
  "buffers": [{{ "byteLength": {len}, "uri": "data:application/octet-stream;base64,{bin}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 48 }},
    {{ "buffer": 0, "byteOffset": 48, "byteLength": 48 }},
    {{ "buffer": 0, "byteOffset": 96, "byteLength": 12, "target": 34963 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 4, "type": "VEC3",
       "min": [-0.5, -0.5, 0], "max": [0.5, 0.5, 0] }},
    {{ "bufferView": 1, "componentType": 5126, "count": 4, "type": "VEC3" }},
    {{ "bufferView": 2, "componentType": 5123, "count": 6, "type": "SCALAR" }}
  ],
  "materials": [
    {{ "pbrMetallicRoughness": {{ "baseColorFactor": [1, 1, 1, 1], "metallicFactor": 0, "roughnessFactor": 1 }} }}
  ],
  "meshes": [
    {{ "primitives": [{{ "attributes": {{ "POSITION": 0, "NORMAL": 1 }}, "indices": 2, "material": 0 }}] }}
  ],
  "nodes": [
    {{ "mesh": 0, "scale": [2, 2, 1] }},
    {{ "mesh": 0, "translation": [0, 0, 1], "scale": [0.4, 0.4, 1] }}
  ],
  "scenes": [{{ "nodes": [0, 1] }}],
  "scene": 0
}}"#,
        len = bin.len(),
        bin = b64(&bin),
    );
    format!("data:model/gltf+json;base64,{}", b64(json.as_bytes()))
}

// The floor's brightness (red) at x = ±0.5, after two frames.
async fn sides(gl: &Gl, scene: &mut Scene, camera: rswebgl_ecs::Entity) -> (u8, u8) {
    wasm_bindgen_futures::JsFuture::from(scene.when_ready())
        .await
        .unwrap();
    scene.render_frame(camera);
    scene.render_frame(camera);
    let read = |x: i32| {
        let mut px = [0u8; 4];
        gl.read_pixels_with_opt_u8_array(
            x,
            SIZE / 2,
            1,
            1,
            Gl::RGBA,
            Gl::UNSIGNED_BYTE,
            Some(&mut px),
        )
        .unwrap();
        px[0]
    };
    // x = −0.5 and +0.5 in a [−1, 1] view 32 pixels wide.
    (read(8), read(24))
}

#[wasm_bindgen_test]
async fn a_shadow_falls_where_the_light_is_blocked() {
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
    scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    // From the right, 0.5 across per unit down: the square's shadow lands
    // at x ≈ −0.7 … −0.3.
    let sun = scene.spawn();
    scene.look_at(sun, &[4.0, 0.0, 8.0], &[0.0, 0.0, 0.0]);
    let light = |cast: bool| {
        js_sys::JSON::parse(&format!(
            r#"{{ "type": "directional", "intensity": 3, "castShadow": {cast} }}"#
        ))
        .unwrap()
    };

    scene.set_light(sun, light(false)).unwrap();
    let (left, right) = sides(&gl, &mut scene, camera).await;
    assert!(
        left > 60 && (left as i32 - right as i32).abs() <= 2,
        "unshadowed: {left} {right}"
    );

    scene.set_light(sun, light(true)).unwrap();
    let (shadowed, lit) = sides(&gl, &mut scene, camera).await;
    assert!(
        lit > 60 && (lit as i32 - right as i32).abs() <= 2,
        "the lit side stays as it was: {lit} (was {right})"
    );
    assert!(
        shadowed < lit / 4,
        "the shadowed side darkens: {shadowed} vs {lit}"
    );
    // The floor and the square are one mesh and material: one instanced
    // draw into the map (drawn again for the count: the last frame kept it).
    scene.invalidate_shadows();
    scene.render_frame(camera);
    assert_eq!(scene.shadow_maps_drawn(), 1);
    assert_eq!(scene.shadow_draw_calls(), 1);

    // Under reverse-Z (where the device has it): the same.
    if scene.set_reverse_z(true) {
        let (shadowed, lit) = sides(&gl, &mut scene, camera).await;
        assert!(
            lit > 60 && shadowed < lit / 4,
            "reverse-Z: {shadowed} vs {lit}"
        );
    }

    // 16-bit maps: the same shadow in half the memory.
    let full = scene.shadow_memory();
    scene.set_shadow_depth_bits(16).unwrap();
    let (shadowed, lit) = sides(&gl, &mut scene, camera).await;
    assert!(
        lit > 60 && shadowed < lit / 4,
        "16-bit: {shadowed} vs {lit}"
    );
    assert_eq!(scene.shadow_memory(), full / 2.0, "16-bit: half the memory");
    assert!(scene.set_shadow_depth_bits(32).is_err());
    scene.destroy();
}

// Cascades: the same square and floor, plus a second copy 100× as large far
// off to the side, so the casters' box is hundreds of units wide. One map
// fitted to it has texels wider than the square, and its shadow blurs
// away; cascades along the view keep a map as wide as what the camera
// sees, and the shadow sharp.
#[wasm_bindgen_test]
async fn cascades_keep_a_small_shadow_in_a_large_scene() {
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
    scene.set_tone_mapping("none").unwrap();
    scene.set_dither(false);
    scene.set_samples(0);
    scene.set_environment_intensity(0.0, 0.0, 0.0);
    scene.set_shadow_map_size(256);
    let camera = scene.spawn();
    let cam =
        js_sys::JSON::parse(r#"{ "type": "orthographic", "ymag": 1, "znear": 0.1, "zfar": 100 }"#)
            .unwrap();
    scene.set_camera(camera, cam).unwrap();
    scene.look_at(camera, &[0.0, 0.0, 5.0], &[0.0, 0.0, 0.0]);
    scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    let big = scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    scene.set_translation(big, 0.0, 300.0, 0.0);
    scene.set_scale(big, 100.0, 100.0, 1.0);
    let sun = scene.spawn();
    scene.look_at(sun, &[4.0, 0.0, 8.0], &[0.0, 0.0, 0.0]);
    let light = |cascades: u32| {
        js_sys::JSON::parse(&format!(
            r#"{{ "type": "directional", "intensity": 3, "castShadow": true,
                 "shadowCascades": {cascades} }}"#
        ))
        .unwrap()
    };

    scene.set_light(sun, light(1)).unwrap();
    let (blurred, _) = sides(&gl, &mut scene, camera).await;
    assert_eq!(scene.shadow_maps(), 1);

    scene.set_light(sun, light(3)).unwrap();
    let (shadowed, lit) = sides(&gl, &mut scene, camera).await;
    assert!(scene.shadow_maps() >= 2, "{}", scene.shadow_maps());
    assert!(
        lit > 60 && shadowed < lit / 4,
        "cascades: {shadowed} vs {lit}"
    );
    assert!(
        blurred > shadowed * 2,
        "one map over everything is coarser: {blurred} vs {shadowed}"
    );

    // Debugging the cascades tints the lit floor (it is the farthest of the
    // casters from the eye: the last cascade, blue).
    scene.set_shadow_cascade_debug(true);
    wasm_bindgen_futures::JsFuture::from(scene.when_ready())
        .await
        .unwrap();
    scene.render_frame(camera);
    let mut px = [0u8; 4];
    gl.read_pixels_with_opt_u8_array(
        24,
        SIZE / 2,
        1,
        1,
        Gl::RGBA,
        Gl::UNSIGNED_BYTE,
        Some(&mut px),
    )
    .unwrap();
    assert!(px[2] as u32 > px[0] as u32 * 3 / 2, "tinted blue: {px:?}");
    scene.destroy();
}

// The cache: a map whose light and casters didn't move isn't drawn again,
// and still shadows; moving the light draws it. Frozen: drawn once, then
// kept even as the light moves, until invalidated.
#[wasm_bindgen_test]
async fn still_shadow_maps_are_kept_and_frozen_ones_held() {
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
    scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    let sun = scene.spawn();
    scene.look_at(sun, &[4.0, 0.0, 8.0], &[0.0, 0.0, 0.0]);
    let light =
        js_sys::JSON::parse(r#"{ "type": "directional", "intensity": 3, "castShadow": true }"#)
            .unwrap();
    scene.set_light(sun, light).unwrap();

    // Two frames: the map drawn in the first, kept in the second.
    let (shadowed, lit) = sides(&gl, &mut scene, camera).await;
    assert!(lit > 60 && shadowed < lit / 4, "{shadowed} vs {lit}");
    assert_eq!(scene.shadow_maps(), 1);
    assert_eq!(scene.shadow_maps_drawn(), 0, "nothing moved: kept");

    // The light from the left: the shadow moves right; drawn again.
    scene.look_at(sun, &[-4.0, 0.0, 8.0], &[0.0, 0.0, 0.0]);
    scene.render_frame(camera);
    assert_eq!(scene.shadow_maps_drawn(), 1, "the light moved: drawn");
    let (left, right) = sides(&gl, &mut scene, camera).await;
    assert!(right < left / 4, "the shadow follows: {right} vs {left}");

    // Frozen: baked once (fitted to the casters), then held while the
    // light moves back.
    scene.set_shadows_frozen(true);
    // (Here the bake is the map it already holds — a model in view is
    // fitted whole either way — so nothing needs drawing.)
    scene.render_frame(camera);
    scene.look_at(sun, &[4.0, 0.0, 8.0], &[0.0, 0.0, 0.0]);
    let (left, right) = sides(&gl, &mut scene, camera).await;
    assert_eq!(scene.shadow_maps_drawn(), 0, "frozen: held");
    assert!(
        right < left / 4,
        "frozen: still on the right: {right} vs {left}"
    );

    // Invalidated: baked again for the light where it is now.
    scene.invalidate_shadows();
    let (left, right) = sides(&gl, &mut scene, camera).await;
    assert!(left < right / 4, "baked again: {left} vs {right}");
    scene.destroy();
}

// A point light and a spot light over the square: its shadow falls on the
// floor away from the light (a point light: in its −Z face, a quarter of a
// layer).
#[wasm_bindgen_test]
async fn point_and_spot_lights_cast_shadows() {
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
    scene.add_model(load_model(&ctx, &asset_url()).await.expect("load"));
    // At (0.6, 0, 2): the square (|x| ≤ 0.2 at z = 1) shades the floor
    // over x ≈ −1 … −0.2; x = 0.5 sees the light.
    let lamp = scene.spawn();
    scene.look_at(lamp, &[0.6, 0.0, 2.0], &[0.6, 0.0, 0.0]);
    let light = |json: &str| js_sys::JSON::parse(json).unwrap();

    for (name, json) in [
        (
            "point",
            r#"{ "type": "point", "intensity": 40, "castShadow": true }"#,
        ),
        (
            "spot",
            r#"{ "type": "spot", "intensity": 40, "outerConeAngle": 1.2, "castShadow": true }"#,
        ),
    ] {
        scene
            .set_light(lamp, light(&json.replace("true", "false")))
            .unwrap();
        let (left, right) = sides(&gl, &mut scene, camera).await;
        assert!(left > 30 && right > 30, "{name} unshadowed: {left} {right}");
        scene.set_light(lamp, light(json)).unwrap();
        let (shadowed, lit) = sides(&gl, &mut scene, camera).await;
        assert!(
            (lit as i32 - right as i32).abs() <= 3,
            "{name}: the lit side stays: {lit} (was {right})"
        );
        assert!(
            shadowed < left / 4,
            "{name}: shadowed {shadowed} (was {left})"
        );
        let maps = if name == "point" { 6 } else { 1 };
        assert_eq!(scene.shadow_maps(), maps, "{name}");
    }
    scene.destroy();
}
