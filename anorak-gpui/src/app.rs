//! The Anorak search window: search bar with Filter / Sort popovers, a
//! virtualized results table with selection, per-row Grab and Grab selected.

use std::rc::Rc;
use std::cell::RefCell;
use std::time::Duration;

use web_time::Instant;

use gpui::{
    AnyElement, App, ClickEvent, Context, Anchor, Entity, FocusHandle, Focusable, FontWeight,
    KeyBinding, MouseButton, SharedString, Stateful, Subscription, Task,
    UniformListScrollHandle, Window, actions, anchored, canvas, deferred, div, point, prelude::*,
    px, rgb, rgba, transparent_black, uniform_list,
};

use crate::api::{self, ApiItem, QueryResult};
use crate::bench::{self, Bench, Probe, ProbeSlot, ScrollRun};
use crate::icons;
use crate::theme::{self, get as th};
use crate::model::{self, Filters, SortKey, SortLevels, SortSpec};
use crate::text_input::{TextInput, TextInputEvent};

actions!(anorak, [CloseMenus, FocusSearch, FocusNext, FocusPrev]);

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CloseMenus, None),
        KeyBinding::new("secondary-l", FocusSearch, None),
        KeyBinding::new("secondary-f", FocusSearch, None),
        // Cycle through the text fields that are on screen (search, plus the
        // filter fields while that popover is open). In the browser this also
        // keeps Tab from moving DOM focus off GPUI's hidden input element.
        KeyBinding::new("tab", FocusNext, None),
        KeyBinding::new("shift-tab", FocusPrev, None),
    ]);
}

const ROW_H: f32 = 36.;

/// Results table sort headers: (element id, label, field).
const HEADERS: [(&str, &str, SortKey); 4] = [
    ("h-name", "Name", SortKey::Name),
    ("h-seeds", "S/P", SortKey::Seeders),
    ("h-size", "Size", SortKey::Size),
    ("h-date", "Date", SortKey::Date),
];

/// Native uses the system Arial (as the web UI's CSS does); the browser build
/// has no system fonts and uses the IBM Plex Sans it embeds.
#[cfg(not(target_family = "wasm"))]
const UI_FONT: &str = "Arial";
#[cfg(target_family = "wasm")]
const UI_FONT: &str = crate::web::UI_FONT;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Popover {
    Filter,
    Sort,
}

#[derive(Clone, Debug, PartialEq)]
enum GrabState {
    Idle,
    Busy,
    Done,
    Failed(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Status {
    Idle,
    Loading,
    Loaded,
    Error(String),
}

struct Row {
    item: ApiItem,
    selected: bool,
    grab: GrabState,
}

impl Row {
    fn selectable(&self) -> bool {
        !self.item.already_added && !self.item.magnet.is_empty() && self.grab != GrabState::Done
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Check {
    Off,
    On,
    Mixed,
}

pub struct AnorakApp {
    server: String,
    focus: FocusHandle,
    search: Entity<TextInput>,
    f_name: Entity<TextInput>,
    f_min_seeds: Entity<TextInput>,
    f_min_size: Entity<TextInput>,
    f_max_size: Entity<TextInput>,
    _subs: Vec<Subscription>,

    rows: Vec<Row>,
    names_lower: Vec<String>,
    view: Vec<usize>,
    filters: Filters,
    sort: SortLevels,

    popover: Option<Popover>,
    popover_closed_at: Option<(Popover, Instant)>,
    status: Status,
    expanded: Option<usize>,
    grab_selected_busy: bool,
    grabbed_msg: Option<String>,
    scroll: UniformListScrollHandle,
    search_task: Option<Task<()>>,

    bench: Option<Rc<Bench>>,
    probe: ProbeSlot,
    scroll_run: Rc<RefCell<ScrollRun>>,
    first_frame_logged: bool,
    /// `?trace=1` (web): publish `snapshot()` to `window.__anorakState` on
    /// every frame so a harness driving real mouse input can check state.
    #[cfg_attr(not(target_family = "wasm"), allow(dead_code))]
    trace: bool,
}

impl AnorakApp {
    pub fn new(
        server: String,
        bench: Option<Bench>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| TextInput::new("search...", cx).large());
        let f_name = cx.new(|cx| TextInput::new("filter title…", cx));
        let f_min_seeds = cx.new(|cx| TextInput::new("0", cx).numeric());
        let f_min_size = cx.new(|cx| TextInput::new("0", cx).numeric());
        let f_max_size = cx.new(|cx| TextInput::new("∞", cx).numeric());

        let mut subs = vec![cx.subscribe_in(&search, window, |this, _, ev, window, cx| {
            if let TextInputEvent::Submit = ev {
                this.submit_search(window, cx);
            }
        })];
        for input in [&f_name, &f_min_seeds, &f_min_size, &f_max_size] {
            subs.push(cx.subscribe(input, |this, _, ev, cx| {
                if let TextInputEvent::Changed = ev {
                    this.read_filters(cx);
                    this.recompute();
                    cx.notify();
                }
            }));
        }
        window.focus(&search.focus_handle(cx), cx);

        // Follow the system light/dark setting, live.
        theme::set_appearance(window.appearance());
        subs.push(cx.observe_window_appearance(window, |this, window, cx| {
            if theme::set_appearance(window.appearance()) {
                if let Some(b) = &this.bench {
                    b.log(serde_json::json!({"event": "theme_changed", "theme": th().name()}));
                }
                window.refresh();
                cx.notify();
            }
        }));

        let probe: ProbeSlot = Rc::new(RefCell::new(None));
        let bench = bench.map(Rc::new);
        if let Some(b) = &bench {
            *probe.borrow_mut() = Some(Probe {
                name: "first_frame",
                start: b.main_start,
                extra: serde_json::json!({}),
            });
        }

        Self {
            server,
            focus: cx.focus_handle(),
            search,
            f_name,
            f_min_seeds,
            f_min_size,
            f_max_size,
            _subs: subs,
            rows: Vec::new(),
            names_lower: Vec::new(),
            view: Vec::new(),
            filters: Filters::default(),
            sort: SortLevels::default(),
            popover: None,
            popover_closed_at: None,
            status: Status::Idle,
            expanded: None,
            grab_selected_busy: false,
            grabbed_msg: None,
            scroll: UniformListScrollHandle::new(),
            search_task: None,
            bench,
            probe,
            scroll_run: Rc::new(RefCell::new(ScrollRun::default())),
            first_frame_logged: false,
            #[cfg(target_family = "wasm")]
            trace: crate::web::query_param("trace").is_some_and(|v| v != "0"),
            #[cfg(not(target_family = "wasm"))]
            trace: false,
        }
    }

    // ----- data flow -------------------------------------------------------

    fn submit_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let term = self.search.read(cx).text().trim().to_string();
        if term.is_empty() {
            return;
        }
        self.close_menus();
        self.status = Status::Loading;
        self.grabbed_msg = None;
        cx.notify();

        let started = Instant::now();
        let request = api::query_task(self.server.clone(), term, cx);
        self.search_task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            this.update_in(cx, |this, window, cx| {
                this.apply_results(result, started, window, cx)
            })
            .ok();
        }));
    }

    fn apply_results(
        &mut self,
        result: QueryResult,
        started: Instant,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t_apply = Instant::now();
        match result {
            Ok((items, timing)) => {
                self.names_lower = items.iter().map(|i| i.title.to_lowercase()).collect();
                self.rows = items
                    .into_iter()
                    .map(|item| Row {
                        item,
                        selected: false,
                        grab: GrabState::Idle,
                    })
                    .collect();
                self.expanded = None;
                self.status = Status::Loaded;
                self.recompute();
                self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
                #[cfg(target_arch = "wasm32")]
                self.warm_browser_font_fallback(window, cx);
                #[cfg(not(target_arch = "wasm32"))]
                let _ = window;
                if self.bench.is_some() {
                    *self.probe.borrow_mut() = Some(Probe {
                        name: "search_to_render",
                        start: started,
                        extra: serde_json::json!({
                            "http_ms": bench::ms(timing.http.as_secs_f64()),
                            "json_parse_ms": bench::ms(timing.parse.as_secs_f64()),
                            "json_bytes": timing.bytes,
                            "apply_ms": bench::ms(t_apply.elapsed().as_secs_f64()),
                            "rows": self.rows.len(),
                            "visible_rows": self.view.len(),
                        }),
                    });
                }
            }
            Err(err) => {
                self.status = Status::Error(err);
            }
        }
        cx.notify();
    }

    fn read_filters(&mut self, cx: &App) {
        self.filters = Filters {
            name: self.f_name.read(cx).text().to_string(),
            min_seeds: model::parse_num(self.f_min_seeds.read(cx).text()),
            min_mb: model::parse_num(self.f_min_size.read(cx).text()),
            max_mb: model::parse_num(self.f_max_size.read(cx).text()),
        };
    }

    /// Re-filter and re-sort. Rows that get hidden are deselected, as in the web UI.
    fn recompute(&mut self) {
        let rows = &self.rows;
        let view = model::compute_view_by(
            rows.len(),
            |i| &rows[i].item,
            &self.names_lower,
            &self.filters,
            self.sort.specs(),
        );
        let mut visible = vec![false; self.rows.len()];
        for &i in &view {
            visible[i] = true;
        }
        for (i, row) in self.rows.iter_mut().enumerate() {
            if !visible[i] {
                row.selected = false;
            }
        }
        self.view = view;
    }

    fn selection_state(&self) -> (usize, usize) {
        let selectable = self.view.iter().filter(|&&i| self.rows[i].selectable()).count();
        let selected = self
            .view
            .iter()
            .filter(|&&i| self.rows[i].selectable() && self.rows[i].selected)
            .count();
        (selected, selectable)
    }

    fn toggle_select_all(&mut self, cx: &mut Context<Self>) {
        let (selected, selectable) = self.selection_state();
        let new_state = !(selectable > 0 && selected == selectable);
        for &i in &self.view {
            if self.rows[i].selectable() {
                self.rows[i].selected = new_state;
            }
        }
        self.grabbed_msg = None;
        cx.notify();
    }

    fn toggle_row(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows.get_mut(ix) {
            if row.selectable() {
                row.selected = !row.selected;
                self.grabbed_msg = None;
                cx.notify();
            }
        }
    }

    fn grab_one(&mut self, ix: usize, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get_mut(ix) else { return };
        if row.item.magnet.is_empty() || matches!(row.grab, GrabState::Busy | GrabState::Done) {
            return;
        }
        row.grab = GrabState::Busy;
        let (server, magnet, category) =
            (self.server.clone(), row.item.magnet.clone(), row.item.category.clone());
        cx.notify();
        let request = api::grab_task(server, magnet, category, cx);
        cx.spawn(async move |this, cx| {
            let result = request.await;
            this.update(cx, |this, cx| {
                if let Some(row) = this.rows.get_mut(ix) {
                    match result {
                        Ok(()) => {
                            row.grab = GrabState::Done;
                            row.selected = false;
                        }
                        Err(e) => row.grab = GrabState::Failed(e),
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn grab_selected(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<usize> = self
            .view
            .iter()
            .copied()
            .filter(|&i| self.rows[i].selectable() && self.rows[i].selected)
            .collect();
        if targets.is_empty() || self.grab_selected_busy {
            return;
        }
        self.grab_selected_busy = true;
        for &i in &targets {
            self.rows[i].grab = GrabState::Busy;
        }
        cx.notify();
        let jobs: Vec<(usize, String, String)> = targets
            .iter()
            .map(|&i| (i, self.rows[i].item.magnet.clone(), self.rows[i].item.category.clone()))
            .collect();
        let server = self.server.clone();
        cx.spawn(async move |this, cx| {
            let mut ok = 0usize;
            // Sequential, like the web UI, so rqbit sees one add at a time.
            for (ix, magnet, category) in jobs {
                let Ok(request) = this.update(cx, |_, cx| {
                    api::grab_task(server.clone(), magnet, category, cx)
                }) else {
                    return;
                };
                let result = request.await;
                if result.is_ok() {
                    ok += 1;
                }
                let _ = this.update(cx, |this, cx| {
                    if let Some(row) = this.rows.get_mut(ix) {
                        match result {
                            Ok(()) => {
                                row.grab = GrabState::Done;
                                row.selected = false;
                            }
                            Err(e) => row.grab = GrabState::Failed(e),
                        }
                    }
                    cx.notify();
                });
            }
            let _ = this.update(cx, |this, cx| {
                this.grab_selected_busy = false;
                if ok > 0 {
                    this.grabbed_msg = Some(format!("Grabbed {ok}"));
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Column header click: sets sort level 1 (same column flips, a new
    /// column starts at its default direction), as in the web UI.
    fn sort_by_header(&mut self, key: SortKey, cx: &mut Context<Self>) {
        self.sort.header_click(key);
        self.sort_changed(cx);
    }

    fn sort_changed(&mut self, cx: &mut Context<Self>) {
        self.recompute();
        cx.notify();
    }

    fn close_menus(&mut self) {
        self.popover = None;
    }

    fn toggle_popover(&mut self, which: Popover, cx: &mut Context<Self>) {
        // The outside-click handler already closed this popover on mouse-down
        // if the click landed on its own toggle; don't reopen it on mouse-up.
        if let Some((closed, at)) = self.popover_closed_at.take() {
            if closed == which && at.elapsed() < Duration::from_millis(600) {
                cx.notify();
                return;
            }
        }
        let opening = self.popover != Some(which);
        self.close_menus();
        if opening {
            self.popover = Some(which);
        }
        cx.notify();
    }

    fn clear_filters(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for input in [&self.f_name, &self.f_min_seeds, &self.f_min_size, &self.f_max_size] {
            input.update(cx, |i, cx| i.set_text("", cx));
        }
        let _ = window;
        self.read_filters(cx);
        self.recompute();
        cx.notify();
    }

    /// wasm only: gpui_web draws CJK and emoji through Canvas 2D with the browser's
    /// system fonts (CanvasFontFallback::EmojiAndCjk). The first time the page uses each
    /// such script, Chrome resolves a system fallback font synchronously (measured 5-7 ms
    /// per script on Windows, cjkprobe.mjs), which used to land in the scroll frame that
    /// first showed a CJK title: a 20+ ms frame and one dropped frame per bench run.
    /// Shape those titles once, off the results frame, so the lookup and gpui_web's
    /// glyph measurements are cached before the rows scroll into view.
    #[cfg(target_arch = "wasm32")]
    fn warm_browser_font_fallback(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        fn browser_font(c: char) -> bool {
            c >= '\u{2E80}' || ('\u{2600}'..='\u{27BF}').contains(&c)
        }
        let titles: Vec<SharedString> = self
            .rows
            .iter()
            .filter(|r| r.item.title.chars().any(browser_font))
            .take(64)
            .map(|r| SharedString::from(r.item.title.clone()))
            .collect();
        if titles.is_empty() {
            return;
        }
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |_, cx| {
            // After the results frame has been presented.
            executor.timer(Duration::from_millis(50)).await;
            let _ = cx.update(|window, _| {
                let mut style = window.text_style();
                style.font_family = UI_FONT.into();
                // Rows use the root 16 px size; the expanded row's title is bold.
                for weight in [FontWeight::NORMAL, FontWeight::BOLD] {
                    style.font_weight = weight;
                    for title in &titles {
                        let run = style.to_run(title.len());
                        window.text_system().shape_line(title.clone(), px(16.), &[run], None);
                    }
                }
            });
        })
        .detach();
    }

    fn filters_ready(&self) -> bool {
        self.status == Status::Loaded && !self.rows.is_empty()
    }

    // ----- bench script ---------------------------------------------------

    fn start_bench_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.bench.clone() else { return };
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            executor.timer(Duration::from_millis(500)).await;
            // 1. search
            let _ = this.update_in(cx, |this, window, cx| {
                let term = b.term.clone();
                this.search.update(cx, |s, cx| s.set_text(&term, cx));
                this.submit_search(window, cx);
            });
            loop {
                executor.timer(Duration::from_millis(10)).await;
                let done = this
                    .read_with(cx, |this, _| {
                        this.status != Status::Loading && this.probe.borrow().is_none()
                    })
                    .unwrap_or(true);
                if done {
                    break;
                }
            }
            let _ = this.read_with(cx, |_, _| b.log(serde_json::json!({"event": "results_loaded"})));
            // Let the harness sample memory with results on screen.
            executor.timer(Duration::from_millis(1500)).await;

            // 2. filter + re-sort, then back, 5 times each
            for rep in 0..5 {
                for apply in [true, false] {
                    let _ = this.update_in(cx, |this, _window, cx| {
                        let start = Instant::now();
                        let text = if apply { "1080" } else { "" };
                        this.f_name.update(cx, |i, cx| i.set_text(text, cx));
                        this.sort = if apply {
                            SortLevels::from_specs(vec![SortSpec::new(SortKey::Size, false)])
                        } else {
                            SortLevels::default()
                        };
                        this.read_filters(cx);
                        let t = Instant::now();
                        this.recompute();
                        let compute = t.elapsed();
                        *this.probe.borrow_mut() = Some(Probe {
                            name: if apply { "filter_sort_apply" } else { "filter_sort_clear" },
                            start,
                            extra: serde_json::json!({
                                "rep": rep,
                                "compute_ms": bench::ms(compute.as_secs_f64()),
                                "visible_rows": this.view.len(),
                            }),
                        });
                        cx.notify();
                    });
                    loop {
                        executor.timer(Duration::from_millis(10)).await;
                        if this.read_with(cx, |this, _| this.probe.borrow().is_none()).unwrap_or(true) {
                            break;
                        }
                    }
                    executor.timer(Duration::from_millis(150)).await;
                }
            }

            // 3. scripted scroll: 20 px per frame for 120 frames
            let _ = this.update_in(cx, |this, _window, cx| {
                this.scroll.0.borrow().base_handle.set_offset(point(px(0.), px(0.)));
                let mut run = this.scroll_run.borrow_mut();
                *run = ScrollRun::default();
                run.steps_left = 120;
                cx.notify();
            });
            loop {
                executor.timer(Duration::from_millis(20)).await;
                let left = this
                    .read_with(cx, |this, _| this.scroll_run.borrow().steps_left)
                    .unwrap_or(0);
                if left == 0 {
                    break;
                }
            }
            executor.timer(Duration::from_millis(100)).await;
            let _ = this.read_with(cx, |this, _| {
                let run = this.scroll_run.borrow();
                let intervals: Vec<f64> = run
                    .paints
                    .windows(2)
                    .map(|w| bench::ms((w[1] - w[0]).as_secs_f64()))
                    .collect();
                // Frames that missed a vsync (> 25 ms since the previous paint), with
                // the CPU build time of that frame, to tell CPU stalls from GPU/compositor ones.
                let slow: Vec<serde_json::Value> = intervals
                    .iter()
                    .enumerate()
                    .filter(|(_, ms)| **ms > 25.0)
                    .map(|(i, ms)| serde_json::json!({"frame": i + 1, "interval_ms": ms, "build_ms": run.build_ms.get(i + 1)}))
                    .collect();
                b.log(serde_json::json!({
                    "event": "scroll",
                    "slow_frames": slow,
                    "frames": run.paints.len(),
                    "frame_interval_ms": bench::stats(intervals),
                    "frame_build_ms": bench::stats(run.build_ms.clone()),
                    "final_offset_px": f32::from(this.scroll.0.borrow().base_handle.offset().y),
                }));
            });
            let _ = this.read_with(cx, |_, _| b.log(serde_json::json!({"event": "done"})));
        })
        .detach();
    }


    fn snapshot(&self) -> serde_json::Value {
        let (selected, selectable) = self.selection_state();
        let first: Vec<serde_json::Value> = self
            .view
            .iter()
            .take(5)
            .map(|&i| {
                let it = &self.rows[i].item;
                serde_json::json!({"title": it.title, "seeders": it.seeders, "size": it.size,
                    "date": it.date, "grab": format!("{:?}", self.rows[i].grab)})
            })
            .collect();
        // The column header texts (the primary sort's column carries ↑/↓).
        let header: Vec<String> = HEADERS.iter().map(|&(_, t, k)| self.header_text(t, k)).collect();
        serde_json::json!({
            "status": format!("{:?}", self.status),
            "theme": th().name(),
            "popover": format!("{:?}", self.popover),
            "shown": self.view.len(), "total": self.rows.len(),
            "selected": selected, "selectable": selectable,
            "grabbed_msg": self.grabbed_msg,
            "filters_active": self.filters.is_active(),
            "sort": self.sort.specs().iter().map(|s| s.label()).collect::<Vec<_>>(),
            "sort_dot": !self.sort.is_default(),
            "sort_panel": self.sort_panel_state(),
            "header": header,
            "first_rows": first,
        })
    }

    /// What the Sort panel shows (active/disabled fields and tooltips), for logs.
    fn sort_panel_state(&self) -> serde_json::Value {
        let rows: Vec<serde_json::Value> = self
            .sort
            .specs()
            .iter()
            .enumerate()
            .map(|(i, spec)| {
                serde_json::json!({
                    "label": SortLevels::verb(i),
                    "active": spec.key.id(),
                    "disabled": SortKey::ALL.iter()
                        .filter(|k| self.sort.field_disabled(i, **k))
                        .map(|k| k.id()).collect::<Vec<_>>(),
                    "field_tips": SortKey::ALL.iter()
                        .map(|k| self.sort.field_tip(i, *k)).collect::<Vec<_>>(),
                    "dir_icon": if spec.asc { "up" } else { "down" },
                    "dir_tip": self.sort.dir_tip(i),
                    "removable": i > 0,
                })
            })
            .collect();
        serde_json::json!({
            "rows": rows,
            "add_enabled": self.sort.can_add(),
            "add_tip": self.sort.add_tip(),
        })
    }

    /// Functional script: drives the same handlers the mouse/keyboard use and
    /// logs state after each step (with pauses so a harness can screenshot).
    fn start_selftest_script(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(b) = self.bench.clone() else { return };
        let executor = cx.background_executor().clone();
        cx.spawn_in(window, async move |this, cx| {
            macro_rules! step {
                ($name:expr, $pause:expr, |$t:ident, $w:ident, $c:ident| $body:block) => {{
                    let _ = this.update_in(cx, |$t, $w, $c| {
                        let _ = (&$t, &$w);
                        $body;
                        $c.notify();
                    });
                    executor.timer(Duration::from_millis(250)).await;
                    let _ = this.read_with(cx, |t, _| {
                        let mut v = t.snapshot();
                        v["event"] = serde_json::Value::from(format!("step:{}", $name));
                        b.log(v);
                    });
                    executor.timer(Duration::from_millis($pause)).await;
                }};
            }
            executor.timer(Duration::from_millis(400)).await;
            step!("before_search", 0, |t, w, c| {
                t.search.update(c, |s, c| s.set_text(&b.term, c));
            });
            let _ = this.update_in(cx, |t, w, c| t.submit_search(w, c));
            loop {
                executor.timer(Duration::from_millis(20)).await;
                if this.read_with(cx, |t, _| t.status != Status::Loading).unwrap_or(true) {
                    break;
                }
            }
            step!("results", 1200, |t, w, c| {});
            step!("open_filter", 1500, |t, w, c| { t.toggle_popover(Popover::Filter, c); });
            step!("filter_1080_min10_min500mb", 1500, |t, w, c| {
                t.f_name.update(c, |i, c| i.set_text("1080", c));
                t.f_min_seeds.update(c, |i, c| i.set_text("10", c));
                t.f_min_size.update(c, |i, c| i.set_text("500", c));
            });
            step!("escape_closes", 300, |t, w, c| { t.close_menus(); });
            step!("open_sort", 1200, |t, w, c| { t.toggle_popover(Popover::Sort, c); });
            step!("pick_name", 300, |t, w, c| { t.sort.pick(0, SortKey::Name); t.sort_changed(c); });
            step!("flip_row1", 300, |t, w, c| { t.sort.flip(0); t.sort_changed(c); });
            step!("add_row", 300, |t, w, c| { t.sort.add(); t.sort_changed(c); });
            step!("add_two_more", 1200, |t, w, c| { t.sort.add(); t.sort.add(); t.sort_changed(c); });
            step!("add_at_max_noop", 300, |t, w, c| { t.sort.add(); t.sort_changed(c); });
            step!("pick_disabled_noop", 300, |t, w, c| { t.sort.pick(2, SortKey::Name); t.sort_changed(c); });
            step!("swap_row2_date", 300, |t, w, c| { t.sort.pick(1, SortKey::Date); t.sort_changed(c); });
            step!("remove_row3", 300, |t, w, c| { t.sort.remove(2); t.sort_changed(c); });
            step!("toggle_sort_closes", 300, |t, w, c| { t.toggle_popover(Popover::Sort, c); });
            step!("header_click_size", 300, |t, w, c| { t.sort_by_header(SortKey::Size, c); });
            step!("header_click_size_again", 300, |t, w, c| { t.sort_by_header(SortKey::Size, c); });
            step!("header_click_name", 300, |t, w, c| { t.sort_by_header(SortKey::Name, c); });
            step!("clear_filters_and_sort", 300, |t, w, c| {
                t.clear_filters(w, c);
                t.sort.reset();
                t.sort_changed(c);
            });
            step!("select_all", 300, |t, w, c| { t.toggle_select_all(c); });
            step!("unselect_first_row_mixed", 300, |t, w, c| {
                let ix = t.view[0];
                t.toggle_row(ix, c);
            });
            step!("select_all_again_then_none", 300, |t, w, c| {
                t.toggle_select_all(c);
                t.toggle_select_all(c);
            });
            step!("select_rows_0_1_2", 800, |t, w, c| {
                for v in 0..3 { let ix = t.view[v]; t.toggle_row(ix, c); }
            });
            step!("grab_selected", 1500, |t, w, c| { t.grab_selected(c); });
            step!("grab_row_4", 1500, |t, w, c| { let ix = t.view[4]; t.grab_one(ix, c); });
            let _ = this.read_with(cx, |_, _| b.log(serde_json::json!({"event": "done"})));
        })
        .detach();
    }

    // ----- rendering helpers ---------------------------------------------

    fn render_probe(&mut self, window: &mut Window) -> Option<AnyElement> {
        let bench = self.bench.clone()?;
        let has_probe = self.probe.borrow().is_some();
        let scrolling = self.scroll_run.borrow().steps_left > 0;
        if scrolling {
            let mut run = self.scroll_run.borrow_mut();
            let y = f32::from(self.scroll.0.borrow().base_handle.offset().y);
            self.scroll
                .0
                .borrow()
                .base_handle
                .set_offset(point(px(0.), px(y - 20.)));
            run.steps_left -= 1;
            window.request_animation_frame();
        }
        if !has_probe && !scrolling {
            return None;
        }
        let probe = self.probe.clone();
        let scroll_run = self.scroll_run.clone();
        let render_start = Instant::now();
        Some(
            canvas(
                |_, _, _| {},
                move |_, _, _, _| {
                    let now = Instant::now();
                    if let Some(p) = probe.borrow_mut().take() {
                        let mut extra = p.extra;
                        if let Some(obj) = extra.as_object_mut() {
                            obj.insert("event".into(), p.name.into());
                            obj.insert(
                                "elapsed_ms".into(),
                                bench::ms((now - p.start).as_secs_f64()).into(),
                            );
                        }
                        bench.log(extra);
                    }
                    let mut run = scroll_run.borrow_mut();
                    if scrolling {
                        run.paints.push(now);
                        run.build_ms.push(bench::ms((now - render_start).as_secs_f64()));
                    }
                },
            )
            .size_0()
            .into_any_element(),
        )
    }

    fn render_search_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let ready = self.filters_ready();
        let filter_active = self.filters.is_active();
        let sort_active = !self.sort.is_default();
        div()
            .flex()
            .items_center()
            .gap(px(8.))
            .my(px(8.))
            .child(div().flex_1().min_w(px(0.)).child(self.search.clone()))
            .child(self.render_dropdown(Popover::Filter, "Filter", ready, filter_active, cx))
            .child(self.render_dropdown(Popover::Sort, "Sort", ready, sort_active, cx))
    }

    fn render_dropdown(
        &mut self,
        which: Popover,
        label: &'static str,
        enabled: bool,
        active_dot: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let open = self.popover == Some(which);
        let mut toggle = div()
            .id(label)
            .relative()
            .h(px(45.))
            .min_w(px(78.))
            .px(px(14.))
            .flex()
            .items_center()
            .justify_center()
            .border(px(3.))
            .rounded(px(4.))
            .border_color(rgb(if open { th().border_strong } else { th().border }))
            .bg(rgb(if open { th().accent_bg } else { th().surface }))
            .text_size(px(14.))
            .font_weight(FontWeight::BOLD)
            .child(label);
        if active_dot {
            toggle = toggle.child(
                div()
                    .absolute()
                    .top(px(4.))
                    .right(px(4.))
                    .size(px(8.))
                    .rounded_full()
                    .bg(rgb(th().text)),
            );
        }
        if enabled {
            toggle = toggle
                .cursor_pointer()
                .hover(|s| s.border_color(rgb(th().border_strong)))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.toggle_popover(which, cx)
                }));
        } else {
            toggle = toggle.opacity(0.45);
        }

        let mut wrapper = div().relative().flex_none().child(toggle);
        if open {
            let panel = match which {
                Popover::Filter => self.render_filter_panel(cx).into_any_element(),
                Popover::Sort => self.render_sort_panel(cx).into_any_element(),
            };
            wrapper = wrapper.child(
                div().absolute().bottom_0().right_0().child(deferred(
                    anchored()
                        .anchor(Anchor::TopRight)
                        .offset(point(px(0.), px(6.)))
                        .snap_to_window_with_margin(px(8.))
                        .child(
                            div()
                                .id(SharedString::from(format!("{label}-panel")))
                                .occlude()
                                .on_mouse_down_out(cx.listener(move |this, _, _, cx| {
                                    if this.popover == Some(which) {
                                        this.close_menus();
                                        this.popover_closed_at = Some((which, Instant::now()));
                                        cx.notify();
                                    }
                                }))
                                .child(panel),
                        ),
                )),
            );
        }
        wrapper
    }

    fn panel_frame() -> gpui::Div {
        div()
            .w(px(320.))
            .p(px(12.))
            .flex()
            .flex_col()
            .gap(px(10.))
            .bg(rgb(th().accent_bg))
            .border_2()
            .border_color(rgb(th().panel_border))
            .rounded(px(4.))
            .shadow_lg()
            .text_color(rgb(th().text))
    }

    fn field(label: &'static str, input: AnyElement) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .flex_1()
            .min_w(px(0.))
            .child(div().text_size(px(12.)).font_weight(FontWeight::BOLD).child(label))
            .child(input)
    }

    fn render_filter_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        Self::panel_frame()
            .child(Self::field("Name contains", self.f_name.clone().into_any_element()))
            .child(Self::field("Min seeds", self.f_min_seeds.clone().into_any_element()))
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(Self::field("Min size (MB)", self.f_min_size.clone().into_any_element()))
                    .child(Self::field("Max size (MB)", self.f_max_size.clone().into_any_element())),
            )
            .child(
                button("clear-filters", "Clear filters", false)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.clear_filters(window, cx)
                    })),
            )
    }

    fn render_sort_panel(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<gpui::Div> = (0..self.sort.len()).map(|i| self.render_sort_row(i, cx)).collect();
        let can_add = self.sort.can_add();
        let add_tip = SharedString::from(self.sort.add_tip());
        let mut add = div()
            .id("sort-add")
            .h(px(32.))
            .px(px(12.))
            .flex()
            .flex_none()
            .items_center()
            .gap(px(5.))
            .bg(rgb(th().surface))
            .border_2()
            .border_color(rgb(th().border_strong))
            .rounded(px(4.))
            .text_size(px(13.))
            .child(icon(icons::PLUS, 12., th().text))
            .child("Then by")
            .tooltip(move |_, cx| cx.new(|_| Tip(add_tip.clone())).into());
        if can_add {
            add = add
                .cursor_pointer()
                .hover(|s| s.bg(rgb(th().surface_hover)))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    if this.sort.add() {
                        this.sort_changed(cx);
                    }
                }));
        } else {
            add = add.opacity(0.5);
        }
        let clear = button("clear-sort", "Clear sort", false)
            .tooltip(|_, cx| cx.new(|_| Tip(SortLevels::CLEAR_TIP.into())).into())
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.sort.reset();
                this.sort_changed(cx);
            }));
        Self::panel_frame()
            .child(div().flex().flex_col().gap(px(10.)).children(rows))
            .child(div().flex().justify_between().gap(px(8.)).child(add).child(clear))
    }

    /// One sort level: joined field buttons (Name / Size / Seeds / Age), the
    /// direction toggle and, after level 1, a remove button (web: .sort-row).
    fn render_sort_row(&mut self, i: usize, cx: &mut Context<Self>) -> gpui::Div {
        let spec = self.sort.specs()[i];
        let n_fields = SortKey::ALL.len();
        let mut fields = div().flex().flex_none();
        for (k, key) in SortKey::ALL.into_iter().enumerate() {
            let active = spec.key == key;
            let disabled = self.sort.field_disabled(i, key);
            let next_active = SortKey::ALL.get(k + 1) == Some(&spec.key);
            let tip = SharedString::from(self.sort.field_tip(i, key));
            // Joined buttons share one 2px border; the active one draws all
            // four of its sides in its own colour (web: z-index 2).
            let mut b = div()
                .id(SharedString::from(format!("sort-{i}-{}", key.id())))
                .h(px(32.))
                .min_w(px(40.))
                .px(px(10.))
                .flex()
                .items_center()
                .justify_center()
                .border_t_2()
                .border_b_2()
                .border_color(rgb(if active { th().primary_bg } else { th().panel_border }))
                .bg(rgb(if active { th().primary_bg } else { th().surface }))
                .child(icon(field_icon(key), 14., if active { th().primary_text } else { th().text }))
                .tooltip(move |_, cx| cx.new(|_| Tip(tip.clone())).into());
            if k == 0 || active {
                b = b.border_l_2();
            }
            if !next_active {
                b = b.border_r_2();
            }
            if k == 0 {
                b = b.rounded_tl(px(4.)).rounded_bl(px(4.));
            }
            if k + 1 == n_fields {
                b = b.rounded_tr(px(4.)).rounded_br(px(4.));
            }
            if disabled {
                b = b.opacity(0.35);
            } else if !active {
                b = b
                    .cursor_pointer()
                    .hover(|s| s.bg(rgb(th().surface_hover)).border_color(rgb(th().border_strong)))
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        if this.sort.pick(i, key) {
                            this.sort_changed(cx);
                        }
                    }));
            }
            fields = fields.child(b);
        }

        let dir_tip = SharedString::from(self.sort.dir_tip(i));
        let dir = div()
            .id(SharedString::from(format!("sort-{i}-dir")))
            .h(px(32.))
            .min_w(px(40.))
            .px(px(10.))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .bg(rgb(th().surface))
            .border_2()
            .border_color(rgb(th().border_strong))
            .rounded(px(4.))
            .cursor_pointer()
            .hover(|s| s.bg(rgb(th().surface_hover)))
            .child(icon(if spec.asc { icons::ARROW_UP } else { icons::ARROW_DOWN }, 14., th().text))
            .tooltip(move |_, cx| cx.new(|_| Tip(dir_tip.clone())).into())
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                this.sort.flip(i);
                this.sort_changed(cx);
            }));

        let remove = (i > 0).then(|| {
            div()
                .id(SharedString::from(format!("sort-{i}-remove")))
                .ml_auto()
                .h(px(32.))
                .min_w(px(32.))
                .px(px(8.))
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .border_2()
                .border_color(transparent_black())
                .rounded(px(4.))
                .cursor_pointer()
                .hover(|s| s.bg(rgb(th().surface_hover)).border_color(rgb(th().border_strong)))
                .child(icon(icons::XMARK, 14., th().text))
                .tooltip(|_, cx| cx.new(|_| Tip(SortLevels::REMOVE_TIP.into())).into())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    if this.sort.remove(i) {
                        this.sort_changed(cx);
                    }
                }))
        });

        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(
                div()
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .child(SortLevels::verb(i)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(fields)
                    .child(dir)
                    .children(remove),
            )
    }

    fn render_toolbar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (selected, _) = self.selection_state();
        let msg = if self.grab_selected_busy {
            "Grabbing…".to_string()
        } else if let Some(m) = &self.grabbed_msg {
            m.clone()
        } else {
            format!("{selected} selected")
        };
        let enabled = selected > 0 && !self.grab_selected_busy;
        let mut grab = button(
            "grab-selected",
            if self.grab_selected_busy { "Grabbing…" } else { "Grab selected" },
            true,
        );
        if enabled {
            grab = grab.on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.grab_selected(cx)));
        } else {
            grab = grab.opacity(0.5);
        }
        div()
            .flex()
            .items_center()
            .gap(px(10.))
            .mt(px(4.))
            .mb(px(12.))
            .text_size(px(13.))
            .child(
                div()
                    .mr_auto()
                    .child(format!("{} shown / {}", self.view.len(), self.rows.len())),
            )
            .child(div().mr(px(8.)).child(msg))
            .child(grab)
    }

    /// Column header label, with ↑/↓ on the primary sort's column.
    fn header_text(&self, text: &str, key: SortKey) -> String {
        let p = self.sort.primary();
        match (p.key == key, p.asc) {
            (false, _) => text.to_string(),
            (true, true) => format!("{text} ↑"),
            (true, false) => format!("{text} ↓"),
        }
    }

    fn render_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let (selected, selectable) = self.selection_state();
        let all = if selectable == 0 || selected == 0 {
            Check::Off
        } else if selected == selectable {
            Check::On
        } else {
            Check::Mixed
        };
        let head = |id: &'static str, text: String, key: SortKey, cx: &mut Context<Self>| {
            div()
                .id(id)
                .px(px(8.))
                .py(px(12.))
                .cursor_pointer()
                .child(text)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.sort_by_header(key, cx)
                }))
        };
        div()
            .flex()
            .items_center()
            .bg(rgb(th().accent_bg))
            .rounded(px(4.))
            .font_weight(FontWeight::BOLD)
            .child(
                div().w(px(36.)).flex().justify_center().child(
                    checkbox("select-all", all).on_click(
                        cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_select_all(cx)),
                    ),
                ),
            )
            .child(
                head("h-name", self.header_text("Name", SortKey::Name), SortKey::Name, cx)
                    .flex_1()
                    .min_w(px(0.)),
            )
            .child(
                head("h-seeds", self.header_text("S/P", SortKey::Seeders), SortKey::Seeders, cx)
                    .w(px(120.)),
            )
            .child(head("h-size", self.header_text("Size", SortKey::Size), SortKey::Size, cx).w(px(140.)))
            .child(head("h-date", self.header_text("Date", SortKey::Date), SortKey::Date, cx).w(px(160.)))
            .child(div().w(px(76.)))
    }

    fn render_rows(
        &mut self,
        range: std::ops::Range<usize>,
        cx: &mut Context<Self>,
    ) -> Vec<Stateful<gpui::Div>> {
        let mut out = Vec::with_capacity(range.len());
        for vix in range {
            let Some(&ix) = self.view.get(vix) else { continue };
            let row = &self.rows[ix];
            let item = &row.item;
            let expanded = self.expanded == Some(ix);

            let select_cell = div().w(px(36.)).flex().justify_center().when(row.selectable(), |d| {
                d.child(
                    checkbox(("row-check", ix), if row.selected { Check::On } else { Check::Off })
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.toggle_row(ix, cx)
                        })),
                )
            });

            let title: SharedString = item.title.clone().into();
            let name_cell = div()
                .id(("name", ix))
                .flex_1()
                .min_w(px(0.))
                .px(px(8.))
                .overflow_hidden()
                .truncate()
                .cursor_pointer()
                .when(expanded, |d| d.font_weight(FontWeight::BOLD))
                .child(title.clone())
                .tooltip(move |_, cx| cx.new(|_| Tip(title.clone())).into())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.expanded = if this.expanded == Some(ix) { None } else { Some(ix) };
                    cx.notify();
                }));

            let grab_cell: AnyElement = if item.already_added || row.grab == GrabState::Done {
                div().child("✓").into_any_element()
            } else if item.magnet.is_empty() {
                div().text_color(rgb(th().muted)).child("—").into_any_element()
            } else {
                match &row.grab {
                    GrabState::Busy => div().child("…").into_any_element(),
                    GrabState::Failed(err) => {
                        let err: SharedString = err.clone().into();
                        small_button(("grab", ix), "Retry")
                            .text_color(rgb(th().error))
                            .tooltip(move |_, cx| cx.new(|_| Tip(err.clone())).into())
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                this.grab_one(ix, cx)
                            }))
                            .into_any_element()
                    }
                    _ => small_button(("grab", ix), "Grab")
                        .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.grab_one(ix, cx)
                        }))
                        .into_any_element(),
                }
            };

            out.push(
                div()
                    .id(("row", ix))
                    .w_full()
                    .h(px(ROW_H))
                    .flex()
                    .items_center()
                    .when(row.selected, |d| d.bg(rgb(th().row_selected)))
                    .hover(|s| s.bg(rgb(th().row_hover)))
                    .child(select_cell)
                    .child(name_cell)
                    .child(
                        div()
                            .w(px(120.))
                            .px(px(8.))
                            .whitespace_nowrap()
                            .child(format!("{} / {}", item.seeders, item.peers)),
                    )
                    .child(div().w(px(140.)).px(px(8.)).whitespace_nowrap().child(item.size_format.clone()))
                    .child(div().w(px(160.)).px(px(8.)).whitespace_nowrap().child(item.date_format.clone()))
                    .child(div().w(px(76.)).flex().justify_center().child(grab_cell)),
            );
        }
        out
    }
}

struct Tip(SharedString);

impl Render for Tip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .max_w(px(640.))
            .px(px(8.))
            .py(px(4.))
            .bg(rgb(th().surface))
            .border_1()
            .border_color(rgb(th().panel_border))
            .rounded(px(4.))
            .shadow_md()
            .text_size(px(13.))
            .text_color(rgb(th().text))
            .child(self.0.clone())
    }
}

fn button(id: &'static str, label: &'static str, primary: bool) -> Stateful<gpui::Div> {
    let (bg, fg, border): (u32, u32, u32) = if primary {
        (th().primary_bg, th().primary_text, th().primary_bg)
    } else {
        (th().surface, th().text, th().border_strong)
    };
    div()
        .id(id)
        .h(px(32.))
        .px(px(12.))
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .bg(rgb(bg))
        .text_color(rgb(fg))
        .border_2()
        .border_color(rgb(border))
        .rounded(px(4.))
        .text_size(px(13.))
        .cursor_pointer()
        .hover(move |s| s.bg(rgb(if primary { th().primary_hover } else { th().surface_hover })))
        .child(label)
}

/// An icon glyph from the embedded icon font (see icons.rs), `size` px square.
fn icon(glyph: &'static str, size: f32, color: u32) -> gpui::Div {
    div()
        .flex_none()
        .font_family(icons::FAMILY)
        .text_size(px(size))
        .line_height(px(size))
        .text_color(rgb(color))
        .child(glyph)
}

fn field_icon(key: SortKey) -> &'static str {
    match key {
        SortKey::Name => icons::FONT,
        SortKey::Size => icons::WEIGHT,
        SortKey::Seeders => icons::SEEDLING,
        SortKey::Date => icons::CLOCK,
    }
}

fn small_button(id: impl Into<gpui::ElementId>, label: &'static str) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(26.))
        .px(px(8.))
        .flex()
        .items_center()
        .bg(rgb(th().surface))
        .border_1()
        .border_color(rgb(th().border_strong))
        .rounded(px(4.))
        .text_size(px(13.))
        .cursor_pointer()
        .hover(|s| s.bg(rgb(th().accent_bg)))
        .child(label)
}

fn checkbox(id: impl Into<gpui::ElementId>, state: Check) -> Stateful<gpui::Div> {
    let on = state != Check::Off;
    div()
        .id(id)
        .size(px(16.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(px(3.))
        .border_2()
        .border_color(rgb(if on { th().check_bg } else { th().border_strong }))
        .bg(rgb(if on { th().check_bg } else { th().surface }))
        .text_color(rgb(th().check_fg))
        .text_size(px(11.))
        .line_height(px(12.))
        .cursor_pointer()
        .child(match state {
            Check::Off => "",
            Check::On => "✓",
            Check::Mixed => "–",
        })
}

impl Focusable for AnorakApp {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for AnorakApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.first_frame_logged {
            self.first_frame_logged = true;
            match self.bench.as_ref().map(|b| b.selftest) {
                Some(false) => self.start_bench_script(window, cx),
                Some(true) => self.start_selftest_script(window, cx),
                None => {}
            }
        }
        let probe = self.render_probe(window);
        #[cfg(target_family = "wasm")]
        if self.trace {
            crate::web::publish_state(&self.snapshot().to_string());
        }

        let mut body = div()
            .w_full()
            .max_w(px(1200.))
            .h_full()
            .px(px(8.))
            .flex()
            .flex_col()
            .child(self.render_search_bar(cx));

        match &self.status {
            Status::Error(err) => {
                body = body.child(
                    div()
                        .p(px(8.))
                        .text_color(rgb(th().error))
                        .child(format!("Search failed: {err}")),
                );
            }
            Status::Loading if self.rows.is_empty() => {
                body = body.child(div().p(px(8.)).child("Searching…"));
            }
            _ => {}
        }

        if self.status != Status::Idle && !matches!(self.status, Status::Error(_)) {
            if self.rows.is_empty() && self.status == Status::Loaded {
                body = body.child(div().p(px(8.)).child("No results found"));
            } else if !self.rows.is_empty() {
                let count = self.view.len();
                body = body
                    .child(self.render_toolbar(cx))
                    .child(self.render_header(cx))
                    .child(
                        div().flex_1().min_h(px(0.)).child(
                            uniform_list(
                                "results",
                                count,
                                cx.processor(|this, range, _window, cx| this.render_rows(range, cx)),
                            )
                            .track_scroll(&self.scroll)
                            .size_full(),
                        ),
                    );
                if let Some(ix) = self.expanded {
                    if let Some(row) = self.rows.get(ix) {
                        body = body.child(
                            div()
                                .id("expanded-title")
                                .my(px(6.))
                                .p(px(8.))
                                .bg(rgb(th().surface))
                                .rounded(px(4.))
                                .border_1()
                                .border_color(rgb(th().panel_border))
                                .child(row.item.title.clone())
                                .cursor_pointer()
                                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                    this.expanded = None;
                                    cx.notify();
                                })),
                        );
                    }
                }
            }
        }

        if self.status == Status::Loading && !self.rows.is_empty() {
            body = body.child(
                div()
                    .absolute()
                    .bottom(px(8.))
                    .right(px(16.))
                    .px(px(10.))
                    .py(px(4.))
                    .rounded(px(4.))
                    .bg(rgba((th().primary_bg << 8) | 0xDD))
                    .text_color(rgb(th().primary_text))
                    .text_size(px(13.))
                    .child("Searching…"),
            );
        }

        div()
            .id("root")
            .key_context("AnorakApp")
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &CloseMenus, _, cx| {
                this.close_menus();
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| {
                window.focus_next(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|_, _: &FocusPrev, window, cx| {
                window.focus_prev(cx);
                cx.notify();
            }))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                window.focus(&this.search.focus_handle(cx), cx);
                cx.notify();
            }))
            .on_mouse_down(MouseButton::Left, |_, _, _| {})
            .size_full()
            .relative()
            .flex()
            .justify_center()
            .bg(rgb(th().bg))
            .font_family(UI_FONT)
            .text_size(px(16.))
            .text_color(rgb(th().text))
            .child(body)
            .children(probe)
    }
}
