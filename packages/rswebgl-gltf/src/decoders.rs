//! The JS side of the decoders (`decoders.js`): the KTX2 transcoder and
//! the Draco decoder, each in a pool of workers, loaded from the path set
//! for it.

use js_sys::Uint8Array;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/src/decoders.js")]
extern "C" {
    #[wasm_bindgen(js_name = setDecoderPath)]
    pub(crate) fn set_decoder_path(kind: &str, path: &str);
    #[wasm_bindgen(js_name = hasDecoder)]
    pub(crate) fn has_decoder(kind: &str) -> bool;
    #[wasm_bindgen(js_name = ktx2Transcode)]
    pub(crate) fn ktx2_transcode(bytes: &Uint8Array, srgb: bool, config: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(js_name = dracoDecode)]
    pub(crate) fn draco_decode(bytes: &Uint8Array, attributes: &JsValue) -> js_sys::Promise;
}
