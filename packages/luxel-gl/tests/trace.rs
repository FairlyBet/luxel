// The CPU profiler: `tracing` spans and JS-style spans land in the Chrome
// trace JSON with their fields, nothing is kept outside a recording, and a
// recording stops when full. Run with:
// wasm-pack test --headless --chrome packages/luxel-gl
#![cfg(target_arch = "wasm32")]

use wasm_bindgen::JsValue;
use wasm_bindgen_test::*;

use luxel_gl::trace::{self, Trace};

fn names(report: &str) -> Vec<String> {
    let v = js_sys::JSON::parse(report).expect("valid JSON");
    let events = js_sys::Reflect::get(&v, &"traceEvents".into()).unwrap();
    js_sys::Array::from(&events)
        .iter()
        .filter(|e| {
            js_sys::Reflect::get(e, &"ph".into())
                .unwrap()
                .as_string()
                .as_deref()
                != Some("M")
        })
        .map(|e| {
            js_sys::Reflect::get(&e, &"name".into())
                .unwrap()
                .as_string()
                .unwrap()
        })
        .collect()
}

#[wasm_bindgen_test]
fn records_spans_counters_and_stops() {
    // Not recording: nothing kept.
    {
        let _s = tracing::info_span!("before").entered();
    }
    trace::start(1000);
    {
        let _outer = tracing::info_span!("outer", n = 3u32).entered();
        Trace::begin("js span");
        Trace::end();
        trace::counter("value", 1.5);
    }
    tracing::info!("marker");
    trace::stop();
    {
        let _s = tracing::info_span!("after").entered();
    }
    let report = trace::report();
    let n = names(&report);
    assert!(n.contains(&"outer".to_string()), "{n:?}");
    assert!(n.contains(&"js span".to_string()));
    assert!(n.contains(&"value".to_string()));
    assert!(n.contains(&"marker".to_string()));
    assert!(!n.contains(&"before".to_string()));
    assert!(!n.contains(&"after".to_string()));
    assert!(report.contains(r#""args":{"n":3}"#), "{report}");

    // Full: recording stops by itself.
    trace::start(3);
    for _ in 0..10 {
        let _s = tracing::info_span!("many").entered();
    }
    assert!(!trace::is_recording());
    assert_eq!(Trace::event_count(), 3);
    assert!(trace::report().contains(r#""truncated":"true""#));
}

// A non-finite time from JS is dropped, so the report stays valid JSON.
#[wasm_bindgen_test]
fn non_finite_times_keep_the_report_valid() {
    Trace::start(None);
    Trace::complete("nan", f64::NAN, 1.0, 1);
    Trace::complete("inf", 0.0, f64::INFINITY, 1);
    Trace::complete("ok", 0.0, 1.0, 1);
    Trace::stop();
    assert_eq!(names(&Trace::report()), ["ok"]);
}

// GPU frame spans: never negative (a fence set before a restart keeps its
// time on the new clock) and never overlapping on their track.
#[wasm_bindgen_test]
async fn gpu_frame_spans_tile_their_track() {
    // Fences signal only once the page has gone back to its event loop.
    async fn tick() {
        let p = js_sys::Promise::new(&mut |r, _| {
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&r, 5)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(p).await.unwrap();
    }
    let f = crate::common::fixture();
    Trace::start(None);
    for _ in 0..3 {
        Trace::gpu_fence(&f.ctx);
    }
    // Restarted with fences in flight.
    Trace::start(None);
    for _ in 0..6 {
        tick().await;
        Trace::gpu_fence(&f.ctx);
    }
    Trace::stop();
    let v = js_sys::JSON::parse(&Trace::report()).expect("valid JSON");
    let events = js_sys::Array::from(&js_sys::Reflect::get(&v, &"traceEvents".into()).unwrap());
    let num = |e: &JsValue, k: &str| {
        js_sys::Reflect::get(e, &k.into())
            .unwrap()
            .as_f64()
            .unwrap()
    };
    let mut last_end = f64::NEG_INFINITY;
    let mut spans = 0;
    for e in events.iter() {
        let name = js_sys::Reflect::get(&e, &"name".into())
            .unwrap()
            .as_string();
        if name.as_deref() != Some("gpu frame") {
            continue;
        }
        let (ts, dur) = (num(&e, "ts"), num(&e, "dur"));
        assert!(ts >= 0.0 && dur >= 0.0, "ts {ts} dur {dur}");
        assert!(ts >= last_end - 1e-6, "overlap: {ts} < {last_end}");
        last_end = ts + dur;
        spans += 1;
    }
    assert!(spans >= 3, "{spans} gpu frames: {}", Trace::report());
}
