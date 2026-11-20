// Auto-resize of the default framebuffer: the drawing buffer follows the
// canvas's CSS size, and dropping the context stops the ResizeObserver
// (otherwise it calls a dropped closure). Run with:
// wasm-pack test --headless --chrome packages/luxel-gl
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use wasm_bindgen_test::*;
use web_sys::HtmlCanvasElement;

use luxel_gl::context::{Context, ContextOptions};

// Lets the browser lay out and deliver resize observations.
async fn settle() {
    let p: js_sys::Promise = js_sys::eval("new Promise(r => setTimeout(r, 150))")
        .unwrap()
        .unchecked_into();
    JsFuture::from(p).await.unwrap();
}

#[wasm_bindgen_test]
async fn auto_resize_follows_the_canvas_and_stops_with_the_context() {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas
        .set_attribute("style", "display: block; width: 40px; height: 30px")
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    js_sys::eval(
        "window.__resize_errors = []; \
         addEventListener('error', e => __resize_errors.push(String(e.message)))",
    )
    .unwrap();

    let ctx = Context::from_canvas(&canvas, &ContextOptions::new()).expect("ctx");
    let fb = ctx.default_framebuffer();
    fb.enable_auto_resize().expect("auto-resize");
    settle().await;
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    assert_eq!(fb.width(), (40.0 * dpr).round() as i32);
    assert_eq!(canvas.width(), (40.0 * dpr).round() as u32);

    drop(fb);
    drop(ctx);
    canvas
        .set_attribute("style", "display: block; width: 64px; height: 30px")
        .unwrap();
    settle().await;
    let errors = js_sys::eval("__resize_errors.join('; ')").unwrap();
    assert_eq!(errors.as_string().unwrap(), "");
    // Nothing resizes it any more.
    assert_eq!(canvas.width(), (40.0 * dpr).round() as u32);
    canvas.remove();
}

// The drawing buffer at a scale of the device pixels, capped per CSS pixel;
// both apply at once and follow later resizes.
#[wasm_bindgen_test]
async fn resolution_scale_and_pixel_ratio_cap() {
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: HtmlCanvasElement = document
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas
        .set_attribute("style", "display: block; width: 80px; height: 40px")
        .unwrap();
    document.body().unwrap().append_child(&canvas).unwrap();
    let ctx = Context::from_canvas(&canvas, &ContextOptions::new()).expect("ctx");
    let fb = ctx.default_framebuffer();
    fb.enable_auto_resize().expect("auto-resize");
    settle().await;
    let dpr = web_sys::window().unwrap().device_pixel_ratio();
    let device = |css: f64| (css * dpr).round();

    fb.set_resolution_scale(0.5);
    assert_eq!(fb.width() as f64, (device(80.0) * 0.5).round());
    assert_eq!(canvas.height() as f64, (device(40.0) * 0.5).round());
    // A later resize keeps the scale. It lands when the next frame asks
    // for the size, not in the observer (which runs after the frame's
    // drawing: resizing there would show a cleared canvas).
    canvas
        .set_attribute("style", "display: block; width: 120px; height: 40px")
        .unwrap();
    settle().await;
    assert_eq!(
        canvas.width() as f64,
        (device(80.0) * 0.5).round(),
        "not yet"
    );
    assert_eq!(fb.width() as f64, (device(120.0) * 0.5).round());
    assert_eq!(canvas.width() as f64, (device(120.0) * 0.5).round());

    // At most 0.25 pixels per CSS pixel: smaller than the scale here.
    fb.set_resolution_scale(1.0);
    fb.set_max_pixel_ratio(0.25);
    assert_eq!(fb.width() as f64, (device(120.0) * (0.25 / dpr)).round());
    fb.set_max_pixel_ratio(0.0); // no cap
    assert_eq!(fb.width() as f64, device(120.0));
    assert_eq!(fb.max_pixel_ratio(), f64::INFINITY);
    drop(fb);
    drop(ctx);
    canvas.remove();
}

// A context for a WebGL2 context made elsewhere covers its drawing buffer,
// and one Viewport object can be set again (it isn't consumed).
#[wasm_bindgen_test]
fn from_gl_covers_the_drawing_buffer() {
    let canvas: HtmlCanvasElement = web_sys::window()
        .unwrap()
        .document()
        .unwrap()
        .create_element("canvas")
        .unwrap()
        .dyn_into()
        .unwrap();
    canvas.set_width(64);
    canvas.set_height(32);
    let gl: web_sys::WebGl2RenderingContext = canvas
        .get_context("webgl2")
        .unwrap()
        .unwrap()
        .dyn_into()
        .unwrap();
    let ctx = Context::from_gl(gl);
    let fb = ctx.default_framebuffer();
    assert_eq!((fb.width(), fb.height()), (64, 32));
    let vp = luxel_gl::draw::Viewport::new(0, 0, 8, 8);
    fb.set_viewport(&vp);
    fb.set_viewport(&vp);
    assert_eq!(fb.width(), 8);
}
