//! `EXT_texture_webp` / `EXT_texture_avif`: whether this browser decodes
//! WebP and AVIF, probed once by decoding a 1×1 image — ported from
//! three.js' GLTFLoader (r160, `GLTFTextureWebPExtension` /
//! `GLTFTextureAVIFExtension.detectSupport`, MIT), test images included.

use std::cell::Cell;

use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

// three.js' test images: a lossy 1×1 WebP, and a lossy 1×1 AVIF.
const WEBP: &str =
    "data:image/webp;base64,UklGRiIAAABXRUJQVlA4IBYAAAAwAQCdASoBAAEADsD+JaQAA3AAAAAA";
const AVIF: &str = "data:image/avif;base64,AAAAIGZ0eXBhdmlmAAAAAGF2aWZtaWYxbWlhZk1BMUIAAADybWV0YQAAAAAAAAAoaGRscgAAAAAAAAAAcGljdAAAAAAAAAAAAAAAAGxpYmF2aWYAAAAADnBpdG0AAAAAAAEAAAAeaWxvYwAAAABEAAABAAEAAAABAAABGgAAABcAAAAoaWluZgAAAAAAAQAAABppbmZlAgAAAAABAABhdjAxQ29sb3IAAAAAamlwcnAAAABLaXBjbwAAABRpc3BlAAAAAAAAAAEAAAABAAAAEHBpeGkAAAAAAwgICAAAAAxhdjFDgQAMAAAAABNjb2xybmNseAACAAIABoAAAAAXaXBtYQAAAAAAAAABAAEEAQKDBAAAAB9tZGF0EgAKCBgABogQEDQgMgkQAAAAB8dSLfI=";

thread_local! {
    static WEBP_OK: Cell<Option<bool>> = const { Cell::new(None) };
    static AVIF_OK: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Which of WebP and AVIF this browser decodes (each probed on first use).
pub(crate) async fn support() -> (bool, bool) {
    (cached(&WEBP_OK, WEBP).await, cached(&AVIF_OK, AVIF).await)
}

async fn cached(slot: &'static std::thread::LocalKey<Cell<Option<bool>>>, image: &str) -> bool {
    if let Some(ok) = slot.with(Cell::get) {
        return ok;
    }
    let ok = decodes(image).await;
    slot.with(|c| c.set(Some(ok)));
    ok
}

// Whether `src` decodes to a 1-pixel-high image (three.js' test). Without
// a document (a worker) the browser is taken to decode it.
async fn decodes(src: &str) -> bool {
    let Ok(image) = web_sys::HtmlImageElement::new() else {
        return true;
    };
    let done = js_sys::Promise::new(&mut |resolve, _| {
        let resolve = resolve.clone();
        let img = image.clone();
        let settle = Closure::once_into_js(move || {
            let _ = resolve.call1(&JsValue::NULL, &JsValue::from_bool(img.height() == 1));
        });
        image.set_onload(Some(settle.unchecked_ref()));
        image.set_onerror(Some(settle.unchecked_ref()));
    });
    image.set_src(src);
    wasm_bindgen_futures::JsFuture::from(done)
        .await
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}
