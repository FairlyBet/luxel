//! Runtime texture compression of a model's images (`setTextureCompression`):
//! once an image has decoded (and its mips are made) it is compressed on the
//! GPU in place (`rswebgl::texture_compressor`), by how the material reads
//! it (`SLOT_USES`): color and data in R / G / B BC1 (base color BC3 where a
//! material using it blends or masks), maps read in alpha (specular, sheen
//! roughness, diffuse transmission) BC3, normal maps (clearcoat's too) BC5;
//! on phones the ETC2 / EAC counterparts. The anisotropy map stays as it is:
//! BC1's channel crosstalk would bend its direction.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rswebgl::context::Context;
use rswebgl::ids::TextureId;
use rswebgl::texture_compressor::{CompressKind, TextureCompressor};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    // Compressed textures by format: how many, milliseconds.
    static STATS: RefCell<std::collections::BTreeMap<String, (u32, f64)>> =
        const { RefCell::new(std::collections::BTreeMap::new()) };
}

/// Whether models loaded from now on compress their textures on the GPU
/// after they decode (on by default): a 2048² map with mips goes from ~21 MB
/// to ~2.8 MB (BC1: base color, emissive, occlusion / metallic-roughness)
/// or ~5.6 MB (BC3: base color with alpha; BC5: normal maps), at some
/// loss — blocks of 4×4 texels share a few colors. Phones without S3TC get
/// ETC2 / EAC instead (ETC2 RGB, ETC2 RGBA, RG11), the same sizes; where
/// neither exists textures stay as they are. Off: full-quality RGBA8.
#[wasm_bindgen(js_name = setTextureCompression)]
pub fn set_texture_compression(on: bool) {
    ENABLED.with(|e| e.set(on));
}

/// See `setTextureCompression`.
#[wasm_bindgen(js_name = textureCompression)]
pub fn texture_compression() -> bool {
    ENABLED.with(Cell::get)
}

// The context's compressor, made on first use (None when compression is off
// or unavailable).
fn compressor(ctx: &Context) -> Option<Rc<TextureCompressor>> {
    if !texture_compression() {
        return None;
    }
    match TextureCompressor::shared(ctx) {
        Ok(comp) => Some(comp),
        Err(e) => {
            crate::fetch::warn(format!("texture compression unavailable: {e}"));
            None
        }
    }
}

/// How a material slot (`material::MAP_SLOTS` order) reads its map, which
/// picks the format: color or data in R / G / B, alpha too, a tangent-space
/// normal, or none (left uncompressed).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Use {
    Rgb,
    Rgba,
    Normal,
    Keep,
}

const SLOT_USES: [Use; crate::material::MAP_COUNT] = [
    Use::Rgb,    // baseColor (Rgba where its material blends or masks)
    Use::Rgb,    // metallicRoughness: G, B
    Use::Normal, // normal
    Use::Rgb,    // occlusion: R
    Use::Rgb,    // emissive
    Use::Rgb,    // clearcoat: R
    Use::Rgb,    // clearcoatRoughness: G
    Use::Normal, // clearcoatNormal
    Use::Rgb,    // transmission: R
    Use::Rgb,    // thickness: G
    Use::Rgba,   // specular: A
    Use::Rgb,    // specularColor
    Use::Rgb,    // sheenColor
    Use::Rgba,   // sheenRoughness: A
    Use::Rgb,    // iridescence: R
    Use::Rgb,    // iridescenceThickness: G
    Use::Keep,   // anisotropy: a direction in R, G (BC1's channel crosstalk would bend it)
    Use::Rgba,   // diffuseTransmission: A
    Use::Rgb,    // diffuseTransmissionColor
];

/// The format each of `doc`'s textures compresses to, by how its materials
/// read it (every slot that uses it must agree: color with and without
/// alpha make color with alpha); None to leave it be.
pub(crate) fn kinds(doc: &gltf::Document) -> Vec<Option<CompressKind>> {
    let mut uses: Vec<Option<Use>> = vec![None; doc.textures().len()];
    let mut merge = |i: usize, u: Use| {
        let Some(slot) = uses.get_mut(i) else {
            return;
        };
        *slot = Some(match (*slot, u) {
            (None, u) => u,
            (Some(a), b) if a == b => a,
            (Some(Use::Rgb), Use::Rgba) | (Some(Use::Rgba), Use::Rgb) => Use::Rgba,
            _ => Use::Keep,
        });
    };
    for mat in doc.materials() {
        let blends = mat.alpha_mode() != gltf::material::AlphaMode::Opaque;
        for (slot, m) in crate::material::doc_maps(&mat).into_iter().enumerate() {
            let Some(m) = m else { continue };
            let u = match SLOT_USES[slot] {
                Use::Rgb if slot == crate::material::BASE_COLOR_MAP && blends => Use::Rgba,
                u => u,
            };
            merge(m.texture, u);
        }
    }
    uses.into_iter()
        .map(|u| match u {
            Some(Use::Rgb) => Some(CompressKind::Rgb),
            Some(Use::Rgba) => Some(CompressKind::Rgba),
            Some(Use::Normal) => Some(CompressKind::Rg),
            _ => None,
        })
        .collect()
}

/// A texture's `on_load(width, height)`: compresses it as `kind` once it
/// has loaded (when compression is on), after `then`. Set the texture into
/// the returned cell once it exists (loads always finish asynchronously).
pub(crate) fn after_load(
    ctx: &Context,
    kind: Option<CompressKind>,
    then: js_sys::Function,
) -> (js_sys::Function, Rc<Cell<TextureId>>) {
    let tex = Rc::new(Cell::new(TextureId::NULL));
    let Some((kind, comp)) = kind.zip(compressor(ctx)) else {
        return (then, tex);
    };
    if !comp.supports(kind) {
        return (then, tex);
    }
    let ctx = ctx.clone();
    let cell = tex.clone();
    let f = Closure::once_into_js(move |w: f64, h: f64| {
        let _ = then.call2(&JsValue::NULL, &w.into(), &h.into());
        if w > 0.0 && h > 0.0 {
            compress_when_ready(ctx, comp, cell.get(), kind);
        }
    });
    (f.unchecked_into(), tex)
}

/// Compresses `tex` as `kind` once the encoders have compiled (when
/// compression is on and the device supports it).
pub(crate) fn compress_later(ctx: &Context, tex: TextureId, kind: CompressKind) {
    if let Some(comp) = compressor(ctx).filter(|c| c.supports(kind)) {
        compress_when_ready(ctx.clone(), comp, tex, kind);
    }
}

fn compress_when_ready(
    ctx: Context,
    comp: Rc<TextureCompressor>,
    tex: TextureId,
    kind: CompressKind,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let _ = wasm_bindgen_futures::JsFuture::from(comp.when_ready()).await;
        if ctx.texture_is_valid(tex) {
            let t0 = js_sys::Date::now();
            if comp.compress(tex, kind) {
                let ms = js_sys::Date::now() - t0;
                let name = comp.format_name(kind).unwrap_or_default();
                STATS.with(|s| {
                    let mut s = s.borrow_mut();
                    let e = s.entry(name).or_default();
                    e.0 += 1;
                    e.1 += ms;
                });
            }
        }
    });
}

/// The textures compressed so far on this page, by format, with the time
/// spent (on the CPU's side; the GPU work runs on after): "ETC2 RGB ×2,
/// RG11 EAC ×1 in 140 ms", or "none".
#[wasm_bindgen(js_name = textureCompressionStats)]
pub fn texture_compression_stats() -> String {
    STATS.with(|s| {
        let s = s.borrow();
        if s.is_empty() {
            return "none".to_string();
        }
        let ms: f64 = s.values().map(|e| e.1).sum();
        let parts: Vec<String> = s.iter().map(|(k, e)| format!("{k} ×{}", e.0)).collect();
        format!("{} in {ms:.0} ms", parts.join(", "))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Texture i over image 0: what each slot's reading makes of it.
    fn kinds_of(materials: &str, textures: usize) -> Vec<Option<CompressKind>> {
        let textures: Vec<String> = (0..textures).map(|_| r#"{ "source": 0 }"#.into()).collect();
        let json = format!(
            r#"{{ "asset": {{ "version": "2.0" }},
                 "extensionsUsed": ["KHR_materials_sheen", "KHR_materials_clearcoat", "KHR_materials_anisotropy"],
                 "images": [{{ "uri": "a.png" }}],
                 "textures": [{}],
                 "materials": [{materials}] }}"#,
            textures.join(",")
        );
        let doc = gltf::Gltf::from_slice(json.as_bytes())
            .expect("gltf")
            .document;
        kinds(&doc)
    }

    #[test]
    fn slots_pick_their_formats() {
        let k = kinds_of(
            r#"{ "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } },
                 "normalTexture": { "index": 1 },
                 "extensions": {
                   "KHR_materials_sheen": { "sheenColorTexture": { "index": 2 }, "sheenRoughnessTexture": { "index": 3 } },
                   "KHR_materials_clearcoat": { "clearcoatNormalTexture": { "index": 4 } },
                   "KHR_materials_anisotropy": { "anisotropyTexture": { "index": 5 } } } },
               { "alphaMode": "BLEND", "pbrMetallicRoughness": { "baseColorTexture": { "index": 6 } } },
               { "normalTexture": { "index": 7 }, "emissiveTexture": { "index": 7 } }"#,
            8,
        );
        use CompressKind::*;
        assert_eq!(
            k,
            vec![
                Some(Rgb),  // base color, opaque
                Some(Rg),   // normal
                Some(Rgb),  // sheen color
                Some(Rgba), // sheen roughness: alpha
                Some(Rg),   // clearcoat normal
                None,       // anisotropy
                Some(Rgba), // base color of a blended material
                None,       // a normal map also read as emissive: at odds
            ]
        );
    }
}
