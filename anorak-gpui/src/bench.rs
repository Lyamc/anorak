//! `--bench` / `?bench=1` instrumentation: timestamps are taken inside the
//! process (paint of the frame that first contains the change). Native builds
//! append JSON lines to the bench log file; the web build pushes the same JSON
//! strings onto `window.__anorakBench` (and the console) for the harness.

use std::cell::RefCell;
use std::rc::Rc;

use web_time::{Instant, SystemTime, UNIX_EPOCH};

pub struct Bench {
    #[cfg(not(target_family = "wasm"))]
    pub out: std::path::PathBuf,
    pub term: String,
    pub main_start: Instant,
    /// Run the functional self-test script instead of the timing script.
    pub selftest: bool,
}

impl Bench {
    pub fn log(&self, json: serde_json::Value) {
        let mut v = json;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("epoch_ms".into(), epoch_ms().into());
            obj.insert(
                "since_main_ms".into(),
                ms(self.main_start.elapsed().as_secs_f64()).into(),
            );
            #[cfg(target_family = "wasm")]
            if let Some(perf) = web_sys::window().and_then(|w| w.performance()) {
                // Relative to navigation start, so the harness gets load -> frame.
                obj.insert("perf_now_ms".into(), perf.now().into());
            }
        }
        #[cfg(not(target_family = "wasm"))]
        {
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.out)
            {
                let _ = writeln!(f, "{v}");
            }
        }
        #[cfg(target_family = "wasm")]
        {
            use wasm_bindgen::JsValue;
            let line = v.to_string();
            if let Some(w) = web_sys::window() {
                let key = JsValue::from_str("__anorakBench");
                let arr = js_sys::Reflect::get(&w, &key)
                    .ok()
                    .filter(|a| js_sys::Array::is_array(a))
                    .map(|a| js_sys::Array::from(&a))
                    .unwrap_or_else(|| {
                        let a = js_sys::Array::new();
                        let _ = js_sys::Reflect::set(&w, &key, &a);
                        a
                    });
                arr.push(&JsValue::from_str(&line));
            }
            web_sys::console::log_1(&format!("ANORAK_BENCH {line}").into());
        }
    }
}

pub fn epoch_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64() * 1000.0)
        .unwrap_or(0.0)
}

pub fn ms(secs: f64) -> f64 {
    (secs * 1_000_000.0).round() / 1000.0
}

/// A pending measurement: logged from the paint of the next frame.
pub struct Probe {
    pub name: &'static str,
    pub start: Instant,
    pub extra: serde_json::Value,
}

pub type ProbeSlot = Rc<RefCell<Option<Probe>>>;

/// State for the scripted scroll run (one step per frame).
#[derive(Default)]
pub struct ScrollRun {
    pub steps_left: usize,
    pub paints: Vec<Instant>,
    pub build_ms: Vec<f64>,
}

pub fn stats(mut v: Vec<f64>) -> serde_json::Value {
    if v.is_empty() {
        return serde_json::Value::Null;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let pct = |p: f64| v[((v.len() as f64 - 1.0) * p).round() as usize];
    let mean = v.iter().sum::<f64>() / v.len() as f64;
    serde_json::json!({
        "n": v.len(),
        "mean": (mean * 1000.0).round() / 1000.0,
        "p50": pct(0.5),
        "p95": pct(0.95),
        "max": v[v.len() - 1],
    })
}
