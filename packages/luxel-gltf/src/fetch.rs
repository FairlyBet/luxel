//! Where a load's files come from, and what's still loading.
//!
//! Every request of a load — the file, its buffers, its images, also those
//! that arrive after `loadModel` resolves — goes through one [`Fetcher`]
//! (`loader.js`): the load's `AbortSignal`, request options, URL modifier,
//! dropped files and progress apply to all of them. [`Loading`] counts the
//! images still on their way and collects what went wrong
//! (`Model::when_loaded`, `Model::warnings`).

use std::cell::RefCell;
use std::rc::Rc;

use luxel_gl::console;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::JsFuture;

#[wasm_bindgen(module = "/src/loader.js")]
extern "C" {
    /// A load's requests (`loader.js`).
    #[derive(Clone)]
    pub type Fetcher;

    // Throws for options it can't take (a signal that isn't one, ...).
    #[wasm_bindgen(js_name = makeFetcher, catch)]
    pub(crate) fn make_fetcher(options: &JsValue) -> Result<Fetcher, JsValue>;

    #[wasm_bindgen(method)]
    fn main(this: &Fetcher, source: &JsValue, base_url: &JsValue) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn bytes(this: &Fetcher, url: &str) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    fn blob(this: &Fetcher, url: &str, mime: &str) -> js_sys::Promise;
    #[wasm_bindgen(method)]
    pub(crate) fn aborted(this: &Fetcher) -> bool;
    #[wasm_bindgen(method, js_name = abortReason)]
    pub(crate) fn abort_reason(this: &Fetcher) -> JsValue;
    #[wasm_bindgen(method)]
    pub(crate) fn cancel(this: &Fetcher);
    #[wasm_bindgen(method, js_name = forReload)]
    pub(crate) fn for_reload(this: &Fetcher) -> Fetcher;
}

impl Fetcher {
    /// The main file: its bytes, the base its URIs resolve against, and its
    /// name (the URL, a file's name, or empty for bytes).
    pub(crate) async fn main_file(
        &self,
        source: &JsValue,
        base_url: &JsValue,
    ) -> Result<(Vec<u8>, String, String), JsValue> {
        let got = JsFuture::from(self.main(source, base_url)).await?;
        let get = |k: &str| js_sys::Reflect::get(&got, &k.into());
        let bytes = as_bytes(get("bytes")?).to_vec();
        let base = get("base")?.as_string().unwrap_or_default();
        let name = get("name")?.as_string().unwrap_or_default();
        Ok((bytes, base, name))
    }

    /// Starts fetching `url`'s bytes now (awaiting it later doesn't hold the
    /// others back).
    pub(crate) fn start_bytes(
        &self,
        url: &str,
    ) -> impl Future<Output = Result<Vec<u8>, JsValue>> + use<> {
        let pending = JsFuture::from(self.bytes(url));
        async move {
            let v = pending.await?;
            Ok(as_bytes(v).to_vec())
        }
    }

    /// `url` as a `Blob` (typed `mime` if the server says no image type).
    pub(crate) async fn fetch_blob(&self, url: &str, mime: &str) -> Result<web_sys::Blob, JsValue> {
        JsFuture::from(self.blob(url, mime)).await?.dyn_into()
    }
}

/// Resolve `rel` against absolute `base` (`new URL(rel, base)`).
pub fn resolve_url(base: &str, rel: &str) -> Result<String, JsValue> {
    let u = web_sys::Url::new_with_base(rel, base)
        .map_err(|_| JsValue::from_str(&format!("bad URL \"{rel}\" (base \"{base}\")")))?;
    Ok(u.href())
}

/// A JS error's message.
pub(crate) fn message(e: &JsValue) -> String {
    if let Some(s) = e.as_string() {
        return s;
    }
    if let Some(e) = e.dyn_ref::<js_sys::Error>() {
        return e.message().into();
    }
    js_sys::Reflect::get(e, &"message".into())
        .ok()
        .and_then(|m| m.as_string())
        .unwrap_or_else(|| format!("{e:?}"))
}

thread_local! {
    // The load whose model is being built (synchronously), for `warn`.
    static BUILDING: RefCell<Option<Rc<Loading>>> = const { RefCell::new(None) };
}

/// While it lives, `warn` reports into `loading` (the model being built).
pub(crate) struct Building(Option<Rc<Loading>>);

impl Building {
    pub(crate) fn set(loading: &Rc<Loading>) -> Self {
        Self(BUILDING.with(|b| b.replace(Some(loading.clone()))))
    }
}

impl Drop for Building {
    fn drop(&mut self) {
        BUILDING.with(|b| *b.borrow_mut() = self.0.take());
    }
}

/// Something left out or worked around while building a model, the load
/// going on: in its `warnings()` and the console (the console alone outside
/// a build).
pub(crate) fn warn(message: String) {
    match BUILDING.with(|b| b.borrow().clone()) {
        Some(loading) => loading.warn(message),
        None => console::warn(&format!("[luxel-gltf] {message}")),
    }
}

/// What a load still waits for, and what went wrong: shared by the model and
/// every image callback of its load.
#[derive(Default)]
pub struct Loading {
    state: RefCell<LoadingState>,
}

#[derive(Default)]
struct LoadingState {
    pending: usize,
    warnings: Vec<String>,
    waiters: Vec<js_sys::Function>,
}

impl Loading {
    pub(crate) fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// A job (an image on its way); done when the returned guard drops.
    pub(crate) fn start(self: &Rc<Self>) -> Job {
        self.state.borrow_mut().pending += 1;
        Job(self.clone())
    }

    /// Something that went wrong but didn't stop the load (also logged).
    pub(crate) fn warn(&self, message: String) {
        console::warn(&format!("[luxel-gltf] {message}"));
        self.state.borrow_mut().warnings.push(message);
    }

    pub(crate) fn warnings(&self) -> Vec<String> {
        self.state.borrow().warnings.clone()
    }

    pub(crate) fn pending(&self) -> usize {
        self.state.borrow().pending
    }

    /// Resolves once nothing is pending (at once if nothing is).
    pub(crate) fn when_done(&self) -> js_sys::Promise {
        let mut resolve_now = None;
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let mut s = self.state.borrow_mut();
            if s.pending == 0 {
                resolve_now = Some(resolve);
            } else {
                s.waiters.push(resolve);
            }
        });
        if let Some(resolve) = resolve_now {
            let _ = resolve.call0(&JsValue::NULL);
        }
        promise
    }

    fn finish(&self) {
        let waiters = {
            let mut s = self.state.borrow_mut();
            s.pending -= 1;
            if s.pending > 0 {
                return;
            }
            std::mem::take(&mut s.waiters)
        };
        for w in waiters {
            let _ = w.call0(&JsValue::NULL);
        }
    }
}

/// A pending job of a [`Loading`]: finished when dropped.
pub struct Job(Rc<Loading>);

impl Job {
    pub(crate) fn loading(&self) -> &Rc<Loading> {
        &self.0
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        self.0.finish();
    }
}

/// A load in progress: its requests, the base its URIs resolve against, and
/// what it still waits for.
#[derive(Clone)]
pub struct Load {
    pub(crate) fetcher: Fetcher,
    pub(crate) base: String,
    pub(crate) loading: Rc<Loading>,
}

// A loader result as bytes: a Uint8Array as it is (`new Uint8Array(array)`
// would copy it once more), anything else (an ArrayBuffer) viewed.
fn as_bytes(v: JsValue) -> js_sys::Uint8Array {
    v.dyn_into::<js_sys::Uint8Array>()
        .unwrap_or_else(|v| js_sys::Uint8Array::new(&v))
}
