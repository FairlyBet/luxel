// Where a model's files come from and how its load goes: from bytes and
// dropped files, a URL modifier, an abort, progress, "fully loaded" and
// warnings — through `loadModel`'s JS options, as an app passes them.
// Run with: wasm-pack test --headless --chrome packages/rswebgl-gltf
#![cfg(target_arch = "wasm32")]

use base64::Engine;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;

use rswebgl::context::{Context, ContextOptions};
use rswebgl_gltf::{LoadRequest, Model, load_model_js, load_request};

// A 1×1 PNG.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn context() -> Context {
    let canvas: web_sys::HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    Context::from_canvas(&canvas, &ContextOptions::new()).expect("ctx")
}

fn b64(b: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(b)
}

// One triangle (positions + indices in `model.bin`), textured with `tex.png`.
// `bounds`: the POSITION accessor's min / max (exporters sometimes drop them).
fn bin() -> Vec<u8> {
    let mut bin = Vec::new();
    for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0] {
        bin.extend_from_slice(&v.to_le_bytes());
    }
    for i in [0u16, 1, 2, 0] {
        bin.extend_from_slice(&i.to_le_bytes());
    }
    bin
}

fn gltf(buffer_uri: &str, image_uri: &str, bounds: bool) -> String {
    let bounds = if bounds {
        r#", "min": [0, 0, 0], "max": [1, 1, 0]"#
    } else {
        ""
    };
    format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": 44, "uri": "{buffer_uri}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }},
    {{ "buffer": 0, "byteOffset": 36, "byteLength": 6, "target": 34963 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3"{bounds} }},
    {{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }}
  ],
  "images": [{{ "uri": "{image_uri}" }}],
  "textures": [{{ "source": 0 }}],
  "materials": [{{ "pbrMetallicRoughness": {{ "baseColorTexture": {{ "index": 0 }} }} }}],
  "meshes": [{{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1, "material": 0 }}] }}],
  "nodes": [{{ "mesh": 0 }}],
  "scenes": [{{ "nodes": [0] }}],
  "scene": 0
}}"#
    )
}

fn png() -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(PNG)
        .unwrap()
}

fn bytes(b: &[u8]) -> JsValue {
    js_sys::Uint8Array::from(b).into()
}

// `new File([bytes], name, { type })`.
fn file(b: &[u8], name: &str, mime: &str) -> JsValue {
    js_sys::Function::new_with_args("b, n, t", "return new File([b], n, { type: t })")
        .call3(&JsValue::NULL, &bytes(b), &name.into(), &mime.into())
        .unwrap()
}

fn object(pairs: &[(&str, JsValue)]) -> JsValue {
    let o = js_sys::Object::new();
    for (k, v) in pairs {
        js_sys::Reflect::set(&o, &(*k).into(), v).unwrap();
    }
    o.into()
}

fn array(items: &[JsValue]) -> JsValue {
    items.iter().collect::<js_sys::Array>().into()
}

async fn load(ctx: &Context, source: JsValue, options: JsValue) -> Model {
    let request = LoadRequest::from_js(source, &options).expect("options");
    load_request(ctx, request).await.expect("load")
}

async fn loaded(model: &Model) {
    JsFuture::from(model.when_loaded()).await.unwrap();
}

// A .gltf picked with its .bin and texture: the URIs resolve to the files.
#[wasm_bindgen_test]
async fn a_gltf_loads_from_its_files() {
    let ctx = context();
    let main = gltf("model.bin", "textures/tex.png", true);
    let files = array(&[
        file(main.as_bytes(), "model.gltf", "model/gltf+json"),
        file(&bin(), "model.bin", ""),
        file(&png(), "tex.png", "image/png"),
    ]);
    let source = file(main.as_bytes(), "model.gltf", "model/gltf+json");
    let model = load(&ctx, source, object(&[("files", files)])).await;
    assert_eq!(model.draw_item_count(), 1);
    assert_eq!(model.source(), "model.gltf");
    loaded(&model).await;
    assert!(model.is_loaded());
    assert_eq!(model.warnings(), Vec::<String>::new());
    // The image arrived: 1×1 storage.
    let tex = model.texture(0);
    assert!(ctx.texture_is_valid(tex));
    model.destroy(&ctx);
}

// A file the model names but wasn't given: the image fails, the model
// still loads, `whenLoaded` resolves and the warning names the image.
#[wasm_bindgen_test]
async fn a_missing_file_is_a_warning() {
    let ctx = context();
    let main = gltf("model.bin", "tex.png", true);
    let files = array(&[file(&bin(), "model.bin", "")]);
    let source = file(main.as_bytes(), "model.gltf", "");
    let model = load(&ctx, source, object(&[("files", files)])).await;
    loaded(&model).await;
    let warnings = model.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("tex.png"), "{warnings:?}");
    model.destroy(&ctx);
}

// What the build leaves out is a warning too: a primitive of bad types
// (COLOR_0 as SCALAR u16, which the spec forbids) is skipped, the rest of
// the model loads.
#[wasm_bindgen_test]
async fn a_skipped_primitive_is_a_warning() {
    let ctx = context();
    let bin_uri = format!("data:application/octet-stream;base64,{}", b64(&bin()));
    let png_uri = format!("data:image/png;base64,{PNG}");
    let main = gltf(&bin_uri, &png_uri, true).replace(
        r#""primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "material": 0 }]"#,
        r#""primitives": [{ "attributes": { "POSITION": 0 }, "indices": 1, "material": 0 }, { "attributes": { "POSITION": 0, "COLOR_0": 1 } }]"#,
    );
    assert!(main.contains(r#"{ "attributes": { "POSITION": 0, "COLOR_0": 1 } }"#));
    let model = load(&ctx, bytes(main.as_bytes()), JsValue::UNDEFINED).await;
    loaded(&model).await;
    assert_eq!(model.draw_item_count(), 1);
    let warnings = model.warnings();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("primitive skipped"), "{warnings:?}");
    model.destroy(&ctx);
}

// Bytes with a base URL whose requests a URL modifier rewrites (here to
// data: URIs, as a signed-URL or proxy function would to other URLs).
#[wasm_bindgen_test]
async fn bytes_resolve_against_base_url_through_the_modifier() {
    let ctx = context();
    let main = gltf("model.bin", "tex.png", true);
    let bin_uri = format!("data:application/octet-stream;base64,{}", b64(&bin()));
    let png_uri = format!("data:image/png;base64,{PNG}");
    let seen = js_sys::Array::new();
    let resolve = js_sys::Function::new_with_args(
        "seen, bin, png, url",
        "seen.push(url); return url.endsWith('.bin') ? bin : png",
    )
    .bind3(&JsValue::NULL, &seen, &bin_uri.into(), &png_uri.into());
    let options = object(&[
        ("baseUrl", "https://cdn.example.invalid/assets/".into()),
        ("resolveUrl", resolve.into()),
    ]);
    let model = load(&ctx, bytes(main.as_bytes()), options).await;
    loaded(&model).await;
    assert_eq!(model.warnings(), Vec::<String>::new());
    let urls: Vec<String> = seen.iter().filter_map(|v| v.as_string()).collect();
    assert!(
        urls.contains(&"https://cdn.example.invalid/assets/model.bin".to_string())
            && urls.contains(&"https://cdn.example.invalid/assets/tex.png".to_string()),
        "{urls:?}"
    );
    model.destroy(&ctx);
}

// Progress: every byte counted, ending with loaded == total.
#[wasm_bindgen_test]
async fn progress_adds_up_the_files() {
    let ctx = context();
    let main = gltf("model.bin", "tex.png", true);
    let files = array(&[
        file(&bin(), "model.bin", ""),
        file(&png(), "tex.png", "image/png"),
    ]);
    let last = js_sys::Array::new();
    let progress = js_sys::Function::new_with_args("last, l, t", "last[0] = l; last[1] = t")
        .bind1(&JsValue::NULL, &last);
    let source = file(main.as_bytes(), "model.gltf", "");
    let options = object(&[("files", files), ("onProgress", progress.into())]);
    let model = load(&ctx, source, options).await;
    loaded(&model).await;
    let total = (main.len() + bin().len() + png().len()) as f64;
    assert_eq!(last.get(0).as_f64(), Some(total));
    assert_eq!(last.get(1).as_f64(), Some(total));
    model.destroy(&ctx);
}

// Aborted: `loadModel` rejects with the signal's reason (an AbortError), as
// fetch does.
#[wasm_bindgen_test]
async fn an_aborted_load_rejects_with_the_reason() {
    let ctx = context();
    let controller = js_sys::Function::new_no_args("return new AbortController()")
        .call0(&JsValue::NULL)
        .unwrap();
    let signal = js_sys::Reflect::get(&controller, &"signal".into()).unwrap();
    let main = gltf("model.bin", "tex.png", true);
    let files = array(&[
        file(&bin(), "model.bin", ""),
        file(&png(), "tex.png", "image/png"),
    ]);
    let promise = load_model_js(
        &ctx,
        file(main.as_bytes(), "model.gltf", ""),
        Some(object(&[("files", files), ("signal", signal)])),
    )
    .expect("starts");
    let abort: js_sys::Function = js_sys::Reflect::get(&controller, &"abort".into())
        .unwrap()
        .dyn_into()
        .unwrap();
    abort.call0(&controller).unwrap();
    let reason = JsFuture::from(promise).await.expect_err("rejected");
    let name = js_sys::Reflect::get(&reason, &"name".into()).unwrap();
    assert_eq!(name.as_string().as_deref(), Some("AbortError"));
}

// The signal given in `request` aborts the same way.
#[wasm_bindgen_test]
async fn a_signal_in_request_aborts_too() {
    let ctx = context();
    let signal = js_sys::Function::new_no_args("return AbortSignal.abort()")
        .call0(&JsValue::NULL)
        .unwrap();
    let main = gltf("model.bin", "tex.png", true);
    let promise = load_model_js(
        &ctx,
        bytes(main.as_bytes()),
        Some(object(&[("request", object(&[("signal", signal)]))])),
    )
    .expect("starts");
    let reason = JsFuture::from(promise).await.expect_err("rejected");
    let name = js_sys::Reflect::get(&reason, &"name".into()).unwrap();
    assert_eq!(name.as_string().as_deref(), Some("AbortError"));
}

// Bytes are the load's own: the app's buffer changing after the call
// doesn't reach the model (or its reload).
#[wasm_bindgen_test]
async fn bytes_are_copied() {
    let ctx = context();
    let main = gltf(
        &format!("data:application/octet-stream;base64,{}", b64(&bin())),
        &format!("data:image/png;base64,{PNG}"),
        true,
    );
    let source = js_sys::Uint8Array::from(main.as_bytes());
    let request = LoadRequest::from_js(source.clone().into(), &JsValue::UNDEFINED).unwrap();
    source.fill(0, 0, source.length());
    let model = load_request(&ctx, request).await.expect("load");
    assert_eq!(model.draw_item_count(), 1);
    model.destroy(&ctx);
}

// A POSITION accessor without min / max (invalid, but common): loaded, with
// a warning; other validation errors still reject.
#[wasm_bindgen_test]
async fn missing_position_bounds_are_a_warning() {
    let ctx = context();
    let main = gltf(
        &format!("data:application/octet-stream;base64,{}", b64(&bin())),
        &format!("data:image/png;base64,{PNG}"),
        false,
    );
    let model = load(&ctx, bytes(main.as_bytes()), JsValue::UNDEFINED).await;
    assert_eq!(model.draw_item_count(), 1);
    let warnings = model.warnings();
    assert!(
        warnings.iter().any(|w| w.contains("min")) && warnings.iter().any(|w| w.contains("max")),
        "{warnings:?}"
    );
    model.destroy(&ctx);

    // Out-of-range indices: an error, not a panic (the gltf crate's own
    // validation indexes POSITION's accessor unchecked).
    for (from, to) in [
        (r#""indices": 1"#, r#""indices": 7"#),
        (r#""POSITION": 0"#, r#""POSITION": 99"#),
    ] {
        let broken = main.replace(from, to);
        let request = LoadRequest::from_js(bytes(broken.as_bytes()), &JsValue::UNDEFINED).unwrap();
        let Err(err) = load_request(&ctx, request).await else {
            panic!("{to}: loaded");
        };
        assert!(err.contains("invalid"), "{err}");
    }
}

// Per-load options win over the module-wide switches, and stay with the
// model (a reload builds it the same).
#[wasm_bindgen_test]
async fn per_load_options_are_kept() {
    let ctx = context();
    let main = gltf(
        &format!("data:application/octet-stream;base64,{}", b64(&bin())),
        &format!("data:image/png;base64,{PNG}"),
        true,
    );
    let options = object(&[
        ("vertexCompression", false.into()),
        ("textureCompression", false.into()),
    ]);
    let model = load(&ctx, bytes(main.as_bytes()), options).await;
    let o = model.load_options();
    assert!(!o.vertex_compression && !o.texture_compression);
    assert!(model.reload_request().is_some());
    let bad = LoadRequest::from_js(bytes(b"{}"), &object(&[("vertexCompression", 1.into())]));
    assert!(bad.is_err());
    model.destroy(&ctx);
}

// One texture as base color (sRGB) and as normal map (data): two GPU
// textures, one per colour space, each map reading its own.
#[wasm_bindgen_test]
async fn a_texture_in_two_colour_spaces_is_made_twice() {
    let ctx = context();
    let main = gltf(
        &format!("data:application/octet-stream;base64,{}", b64(&bin())),
        &format!("data:image/png;base64,{PNG}"),
        true,
    )
    .replace(
        r#""materials": [{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } }]"#,
        r#""materials": [{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } },
                          "normalTexture": { "index": 0 } }]"#,
    );
    assert!(main.contains("normalTexture"));
    let model = load(&ctx, bytes(main.as_bytes()), JsValue::UNDEFINED).await;
    assert_eq!(model.texture_count(), 2);
    let m = &model.materials()[0];
    let slot = |name: &str| {
        rswebgl_gltf::MAP_NAMES
            .iter()
            .position(|n| *n == name)
            .unwrap()
    };
    let (base, normal) = (
        m.map(slot("baseColorTexture")),
        m.map(slot("normalTexture")),
    );
    assert!(base.is_some() && normal.is_some());
    assert_ne!(base.unwrap().0, normal.unwrap().0);
    loaded(&model).await;
    assert_eq!(model.warnings(), Vec::<String>::new());
    model.destroy(&ctx);
}

// A morph target's deltas widen the primitive's bounds (weights 0–1).
#[wasm_bindgen_test]
async fn morph_targets_widen_the_bounds() {
    let ctx = context();
    let mut data = bin(); // positions 0..36, indices 36..42 (+2 pad)
    for d in [0.0f32, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, -3.0, 0.0] {
        data.extend_from_slice(&d.to_le_bytes()); // deltas: 44..80
    }
    let json = format!(
        r#"{{
  "asset": {{ "version": "2.0" }},
  "buffers": [{{ "byteLength": 80, "uri": "data:application/octet-stream;base64,{b}" }}],
  "bufferViews": [
    {{ "buffer": 0, "byteOffset": 0, "byteLength": 36 }},
    {{ "buffer": 0, "byteOffset": 36, "byteLength": 6 }},
    {{ "buffer": 0, "byteOffset": 44, "byteLength": 36 }}
  ],
  "accessors": [
    {{ "bufferView": 0, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, 0, 0], "max": [1, 1, 0] }},
    {{ "bufferView": 1, "componentType": 5123, "count": 3, "type": "SCALAR" }},
    {{ "bufferView": 2, "componentType": 5126, "count": 3, "type": "VEC3", "min": [0, -3, 0], "max": [2, 0, 0] }}
  ],
  "meshes": [{{ "primitives": [{{ "attributes": {{ "POSITION": 0 }}, "indices": 1,
                                  "targets": [{{ "POSITION": 2 }}] }}], "weights": [0] }}],
  "nodes": [{{ "mesh": 0 }}],
  "scenes": [{{ "nodes": [0] }}],
  "scene": 0
}}"#,
        b = b64(&data)
    );
    let model = load(&ctx, bytes(json.as_bytes()), JsValue::UNDEFINED).await;
    let (min, max) = model.items()[0].local_bounds();
    assert_eq!((min, max), ([0.0, -3.0, 0.0], [3.0, 1.0, 0.0]));
    model.destroy(&ctx);
}
