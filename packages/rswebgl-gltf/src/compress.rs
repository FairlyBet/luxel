//! Runtime texture compression of a model's images (`setTextureCompression`):
//! once an image has decoded (and its mips are made) it is compressed on the
//! GPU in place (`rswebgl::texture_compressor`) — base color BC1, or BC3
//! where a material using it blends or masks; emissive, occlusion and
//! metallic-roughness (or the packed ORM) BC1; normal maps BC5. Extension
//! maps (sheen, clearcoat, specular, …) stay as they are: several keep data
//! in alpha or need more precision than BC1 gives.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use rswebgl::console;
use rswebgl::context::Context;
use rswebgl::ids::TextureId;
use rswebgl::texture_compressor::{CompressKind, TextureCompressor};

thread_local! {
    static ENABLED: Cell<bool> = const { Cell::new(true) };
    // One compressor per context (its encoders compile once), remade after
    // a context loss.
    static COMPRESSOR: RefCell<Option<(Context, u32, Rc<TextureCompressor>)>> =
        const { RefCell::new(None) };
}

/// Whether models loaded from now on compress their textures on the GPU
/// after they decode (on by default): a 2048² map with mips goes from ~21 MB
/// to ~2.8 MB (BC1: base color, emissive, occlusion / metallic-roughness)
/// or ~5.6 MB (BC3: base color with alpha; BC5: normal maps), at some
/// loss — blocks of 4×4 texels share two colors. Where the device has no
/// such formats (S3TC / RGTC: most phones, for now) textures stay as they
/// are. Off: full-quality RGBA8.
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
    COMPRESSOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some((c, generation, comp)) = slot.as_ref()
            && c.is_same_context(ctx)
            && *generation == ctx.generation()
        {
            return Some(comp.clone());
        }
        match TextureCompressor::new(ctx) {
            Ok(comp) => {
                let comp = Rc::new(comp);
                *slot = Some((ctx.clone(), ctx.generation(), comp.clone()));
                Some(comp)
            }
            Err(e) => {
                console::error(&format!(
                    "[rswebgl-gltf] texture compression unavailable: {e}"
                ));
                None
            }
        }
    })
}

/// The format each of `doc`'s textures compresses to, by how its materials
/// use it; None to leave it be (an extension's map, or roles at odds).
pub(crate) fn kinds(doc: &gltf::Document) -> Vec<Option<CompressKind>> {
    #[derive(Clone, Copy, Default)]
    struct Use {
        color: bool, // base color, emissive, occlusion, metallic-roughness
        alpha: bool, // base color of a blended or masked material
        normal: bool,
        other: bool,
    }
    let mut uses = vec![Use::default(); doc.textures().len()];
    for mat in doc.materials() {
        let pbr = mat.pbr_metallic_roughness();
        let blends = mat.alpha_mode() != gltf::material::AlphaMode::Opaque;
        if let Some(i) = pbr.base_color_texture().map(|i| i.texture().index()) {
            uses[i].color = true;
            uses[i].alpha |= blends;
        }
        for i in [
            pbr.metallic_roughness_texture()
                .map(|i| i.texture().index()),
            mat.occlusion_texture().map(|i| i.texture().index()),
            mat.emissive_texture().map(|i| i.texture().index()),
        ]
        .into_iter()
        .flatten()
        {
            uses[i].color = true;
        }
        if let Some(i) = mat.normal_texture().map(|i| i.texture().index()) {
            uses[i].normal = true;
        }
        for m in crate::material::doc_maps(&mat)
            .into_iter()
            .skip(5)
            .flatten()
        {
            if let Some(u) = uses.get_mut(m.texture) {
                u.other = true;
            }
        }
    }
    uses.iter()
        .map(|u| match (u.color, u.normal, u.other) {
            (true, false, false) if u.alpha => Some(CompressKind::Rgba),
            (true, false, false) => Some(CompressKind::Rgb),
            (false, true, false) => Some(CompressKind::Rg),
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
            comp.compress(tex, kind);
        }
    });
}
