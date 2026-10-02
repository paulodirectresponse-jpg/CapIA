//! S1 — Preview Surface spike (DESCARTÁVEL). Mede P1 (HWND nativo sob WebView2 transparente) e
//! P2 (SharedBuffer -> canvas) em Windows 11 e escreve um relatório JSON estruturado.
//! Nada aqui é código de produto. Ver tools/s1-preview-spike/README.md.
#![cfg_attr(not(windows), allow(dead_code))]

#[cfg(not(windows))]
fn main() {
    eprintln!("S1 harness: somente Windows 11. Veja tools/s1-preview-spike/README.md.");
    std::process::exit(2);
}

#[cfg(windows)]
mod engines;
#[cfg(windows)]
mod gpu;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    harness::run();
}

#[cfg(windows)]
mod harness {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::{Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    use serde_json::{Map, Value, json};
    use tauri::{Manager, WebviewWindow};
    use windows::Win32::Foundation::HWND;

    use crate::engines::{
        Geometry, LatencySampler, P1Engine, P2Engine, ResourceSampler, SharedMem, read_frame_id,
    };
    use crate::gpu::{Gpu, HEADER_BYTES, Pattern, SubmitLog};
    use crate::win::{self, Rect, ScreenSampler};

    const GRAY: [u8; 3] = [128, 128, 128];
    const OVERLAY_BLEND: [u8; 3] = [191, 64, 64]; // 50% vermelho (255,0,0) sobre cinza 128
    const TOL: i32 = 8;

    fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
        m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn utc_ms() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
    }

    #[derive(Clone, Debug)]
    pub struct Cfg {
        out: String,
        modes: Vec<String>,
        steady_secs: u64,
        p2_res: (u32, u32),
        label: String,
        skip_input: bool,
    }

    fn parse_args() -> Cfg {
        let mut cfg = Cfg {
            out: "s1-harness-report.json".into(),
            modes: vec!["p1".into(), "p1_above".into(), "p2".into()],
            steady_secs: 10,
            p2_res: (960, 540),
            label: String::new(),
            skip_input: false,
        };
        let args: Vec<String> = std::env::args().collect();
        let mut i = 1;
        while i < args.len() {
            let val = args.get(i + 1).cloned().unwrap_or_default();
            match args[i].as_str() {
                "--out" => cfg.out = val,
                "--modes" => cfg.modes = val.split(',').map(str::to_owned).collect(),
                "--steady-secs" => cfg.steady_secs = val.parse().unwrap_or(10),
                "--label" => cfg.label = val,
                "--p2-res" => {
                    if let Some((w, h)) = val.split_once('x') {
                        cfg.p2_res = (w.parse().unwrap_or(960), h.parse().unwrap_or(540));
                    }
                }
                "--skip-input" => {
                    cfg.skip_input = true;
                    i -= 1;
                }
                _ => i -= 1,
            }
            i += 2;
        }
        cfg
    }

    /// Layout reportado pela página (px CSS) convertido em físico no uso.
    #[derive(Clone, Debug, Default)]
    pub struct Layout {
        dpr: f64,
        box_css: [f64; 4],
        overlay_btn_css: [f64; 4],
        toolbar_btn_css: [f64; 4],
        raw: Value,
    }

    #[derive(Default)]
    pub struct Shared {
        layout: Mutex<Layout>,
        seq: Mutex<u64>,
        cv: Condvar,
        events: Mutex<Map<String, Value>>,
        page_stats: Mutex<Option<Value>>,
    }

    impl Shared {
        fn seq(&self) -> u64 {
            *lock(&self.seq)
        }
        fn wait_layout_after(&self, seq0: u64, timeout: Duration) -> Option<Layout> {
            let guard = lock(&self.seq);
            let (g, res) = self
                .cv
                .wait_timeout_while(guard, timeout, |s| *s <= seq0)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            drop(g);
            (!res.timed_out()).then(|| lock(&self.layout).clone())
        }
        fn count(&self, key: &str) -> u64 {
            lock(&self.events)
                .get(key)
                .and_then(Value::as_u64)
                .unwrap_or(0)
        }
    }

    fn rect4(v: &Value, key: &str) -> [f64; 4] {
        let a = &v[key];
        [
            a["x"].as_f64().unwrap_or(0.0),
            a["y"].as_f64().unwrap_or(0.0),
            a["w"].as_f64().unwrap_or(0.0),
            a["h"].as_f64().unwrap_or(0.0),
        ]
    }

    #[tauri::command]
    fn s1_layout(state: tauri::State<'_, Arc<Shared>>, payload: Value) {
        *lock(&state.layout) = Layout {
            dpr: payload["dpr"].as_f64().unwrap_or(1.0),
            box_css: rect4(&payload, "box"),
            overlay_btn_css: rect4(&payload, "overlay_btn"),
            toolbar_btn_css: rect4(&payload, "toolbar_btn"),
            raw: payload,
        };
        *lock(&state.seq) += 1;
        state.cv.notify_all();
    }

    #[tauri::command]
    fn s1_event(state: tauri::State<'_, Arc<Shared>>, kind: String, detail: Value) {
        let mut ev = lock(&state.events);
        let n = ev.get(&kind).and_then(Value::as_u64).unwrap_or(0) + 1;
        ev.insert(kind.clone(), json!(n));
        ev.insert(format!("{kind}__last"), detail);
    }

    #[tauri::command]
    fn s1_page_stats(state: tauri::State<'_, Arc<Shared>>, stats: Value) {
        *lock(&state.page_stats) = Some(stats);
    }

    fn phys(css: [f64; 4], dpr: f64) -> Rect {
        Rect {
            x: (css[0] * dpr).round() as i32,
            y: (css[1] * dpr).round() as i32,
            w: (css[2] * dpr).round() as i32,
            h: (css[3] * dpr).round() as i32,
        }
    }

    /// Mantém COM vivo (o SharedBuffer deve existir enquanto o JS o usa).
    struct KeepAlive(
        #[allow(dead_code)]
        webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2SharedBuffer,
    );
    unsafe impl Send for KeepAlive {}
    static KEEP: Mutex<Vec<KeepAlive>> = Mutex::new(Vec::new());

    #[allow(unsafe_code)]
    fn create_shared_buffer(
        window: &WebviewWindow,
        size: u64,
        meta: &str,
    ) -> Result<SharedMem, String> {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, ICoreWebView2_17,
            ICoreWebView2Environment12,
        };
        use windows::core::{HSTRING, Interface, PCWSTR};
        let (tx, rx) = mpsc::channel();
        let meta = HSTRING::from(meta);
        window
            .with_webview(move |wv| {
                let result = (|| -> Result<SharedMem, String> {
                    let env: ICoreWebView2Environment12 = wv.environment().cast().map_err(|e| {
                        format!("Environment12 indisponível (WebView2 antigo?): {e}")
                    })?;
                    let buf = unsafe { env.CreateSharedBuffer(size) }
                        .map_err(|e| format!("CreateSharedBuffer: {e}"))?;
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    unsafe { buf.Buffer(&mut ptr) }
                        .map_err(|e| format!("SharedBuffer::Buffer: {e}"))?;
                    let core = unsafe { wv.controller().CoreWebView2() }
                        .map_err(|e| format!("CoreWebView2: {e}"))?;
                    let wv17: ICoreWebView2_17 = core
                        .cast()
                        .map_err(|e| format!("ICoreWebView2_17 indisponível: {e}"))?;
                    unsafe {
                        wv17.PostSharedBufferToScript(
                            &buf,
                            COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY,
                            PCWSTR(meta.as_ptr()),
                        )
                    }
                    .map_err(|e| format!("PostSharedBufferToScript: {e}"))?;
                    lock(&KEEP).push(KeepAlive(buf));
                    Ok(SharedMem {
                        ptr,
                        len: size as usize,
                    })
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| format!("with_webview: {e}"))?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|e| format!("timeout criando SharedBuffer: {e}"))?
    }

    struct Ctx {
        app_window: WebviewWindow,
        hwnd: HWND,
        child: HWND,
        shared: Arc<Shared>,
        gpu: Arc<Gpu>,
        pattern: Arc<Mutex<Pattern>>,
        surface: Arc<wgpu::Surface<'static>>,
        log: Arc<SubmitLog>,
        geo: Arc<Mutex<Option<Geometry>>>,
        cfg: Cfg,
        mem: Mutex<Option<SharedMem>>,
    }

    impl Ctx {
        fn js(&self, code: &str) {
            let _ = self.app_window.eval(code);
        }

        /// Pede novo layout à página e espera a resposta.
        fn refresh_layout(&self) -> Result<Layout, String> {
            let s0 = self.shared.seq();
            self.js("window.s1.report()");
            self.shared
                .wait_layout_after(s0, Duration::from_millis(2500))
                .ok_or_else(|| "página não respondeu ao layout".into())
        }

        fn stage_phys(&self, l: &Layout) -> Rect {
            phys(l.box_css, l.dpr)
        }

        fn screen_origin(&self) -> Result<(i32, i32), String> {
            win::client_origin_on_screen(self.hwnd)
        }

        fn set_geometry(&self, l: &Layout, pat: (u32, u32)) -> Result<Rect, String> {
            let r = self.stage_phys(l);
            let (ox, oy) = self.screen_origin()?;
            *lock(&self.geo) = Some(Geometry {
                screen_x: ox + r.x,
                screen_y: oy + r.y,
                w: r.w,
                h: r.h,
                pat_w: pat.0,
                pat_h: pat.1,
            });
            Ok(r)
        }

        fn sample_at_fraction(&self, l: &Layout, fx: f64, fy: f64) -> Result<[u8; 3], String> {
            let r = self.stage_phys(l);
            let (ox, oy) = self.screen_origin()?;
            let x = ox + r.x + (f64::from(r.w) * fx) as i32;
            let y = oy + r.y + (f64::from(r.h) * fy) as i32;
            Ok(win::sample_points(&[(x, y)])?[0])
        }
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        (0..3).all(|i| (i32::from(a[i]) - i32::from(b[i])).abs() <= TOL)
    }

    fn phase<F: FnOnce() -> Result<Value, String>>(name: &str, f: F) -> Value {
        eprintln!("[s1] fase {name} …");
        let start = utc_ms();
        let t0 = Instant::now();
        let outcome = catch_unwind(AssertUnwindSafe(f));
        let (status, body) = match outcome {
            Ok(Ok(v)) => ("ok", v),
            Ok(Err(e)) => ("error", json!({ "error": e })),
            Err(_) => ("panic", json!({ "error": "panic na fase" })),
        };
        json!({ "name": name, "status": status, "started_utc_ms": start, "ended_utc_ms": utc_ms(), "seconds": t0.elapsed().as_secs_f64(), "result": body })
    }

    // ---------------------------------------------------------------- fases (comuns a P1/P2)

    fn probe_static(ctx: &Ctx, mode: &str) -> Result<Value, String> {
        ctx.js("window.s1.setOverlay(false)");
        thread::sleep(Duration::from_millis(500));
        let l = ctx.refresh_layout()?;
        let a = ctx.sample_at_fraction(&l, 0.08, 0.08)?; // dentro da zona cinza, fora do overlay
        let outside = {
            let r = ctx.stage_phys(&l);
            let (ox, oy) = ctx.screen_origin()?;
            win::sample_points(&[(ox + (r.x - 10).max(0), oy + r.y + r.h / 2)])?[0]
        };
        ctx.js("window.s1.setOverlay(true)");
        thread::sleep(Duration::from_millis(600));
        let l2 = ctx.refresh_layout()?;
        let b = ctx.sample_at_fraction(&l2, 0.20, 0.15)?; // zona cinza coberta pelo overlay HTML 50% vermelho
        ctx.js("window.s1.setOverlay(false)");
        Ok(json!({
            "mode": mode,
            "sample_probe_no_overlay_rgb": a, "expected_gray_rgb": GRAY,
            "native_or_canvas_pattern_visible_at_probe": near(a, GRAY),
            "sample_outside_stage_rgb": outside,
            "sample_probe_with_html_overlay_rgb": b, "expected_blend_rgb": OVERLAY_BLEND,
            "html_overlay_composited_over_pattern": near(b, OVERLAY_BLEND),
            "interpretation": "P1: se 'pattern_visible' for false com RGB ≈ fundo da página/janela, o WebView2 transparente NÃO revela o HWND irmão por baixo. Se 'html_overlay_composited' for false, o overlay HTML não aparece sobre o preview (airspace).",
        }))
    }

    fn steady(
        ctx: &Ctx,
        mode: &str,
        pat: (u32, u32),
        stats: &dyn Fn() -> Value,
        with_latency: bool,
    ) -> Result<Value, String> {
        let l = ctx.refresh_layout()?;
        ctx.set_geometry(&l, pat)?;
        ctx.js("window.s1.startStats()");
        let res = ResourceSampler::start();
        let lat = with_latency.then(|| LatencySampler::start(ctx.geo.clone(), ctx.log.clone()));
        thread::sleep(Duration::from_secs(ctx.cfg.steady_secs));
        let lat_json = lat.map(LatencySampler::stop);
        let res_json = res.stop();
        ctx.js("window.s1.stopStats()");
        thread::sleep(Duration::from_millis(400));
        let page = lock(&ctx.shared.page_stats).take();
        Ok(
            json!({ "mode": mode, "seconds": ctx.cfg.steady_secs, "engine_running_stats_at_end_of_phase": stats(), "resources": res_json, "screen_latency": lat_json, "page_stats": page }),
        )
    }

    /// Redimensiona a janela e verifica se o preview (nativo ou canvas) acompanha o layout da página.
    fn resize_phase(
        ctx: &Ctx,
        mode: &str,
        pat: (u32, u32),
        apply: &dyn Fn(Rect, bool),
    ) -> Result<Value, String> {
        let l0 = ctx.refresh_layout()?;
        let dpr = l0.dpr;
        let sizes = [
            (1280, 720),
            (1024, 576),
            (1600, 900),
            (800, 450),
            (1280, 720),
        ];
        let mut steps = Vec::new();
        for (w, h) in sizes {
            let (pw, ph) = (
                (f64::from(w) * dpr).round() as u32,
                (f64::from(h) * dpr).round() as u32,
            );
            let t0 = Instant::now();
            let s0 = ctx.shared.seq();
            ctx.app_window
                .set_size(tauri::PhysicalSize::new(pw, ph))
                .map_err(|e| format!("set_size: {e}"))?;
            let layout = ctx
                .shared
                .wait_layout_after(s0, Duration::from_millis(2500));
            let lag_layout_ms = t0.elapsed().as_secs_f64() * 1e3;
            let l = match layout {
                Some(l) => l,
                None => ctx.refresh_layout()?,
            };
            let r = ctx.set_geometry(&l, pat)?;
            apply(r, false);
            // espera até a amostra do probe aparecer correta (ou 1,5 s)
            let (mut ok_after_ms, mut last) = (None, [0u8; 3]);
            while t0.elapsed() < Duration::from_millis(1500) {
                last = ctx.sample_at_fraction(&l, 0.08, 0.08)?;
                if near(last, GRAY) {
                    ok_after_ms = Some(t0.elapsed().as_secs_f64() * 1e3);
                    break;
                }
                thread::sleep(Duration::from_millis(5));
            }
            steps.push(json!({
                "requested_physical_size": [pw, ph], "stage_physical_rect": [r.x, r.y, r.w, r.h],
                "layout_report_lag_ms": lag_layout_ms, "pattern_correct_after_ms": ok_after_ms, "last_probe_rgb": last,
            }));
        }
        // Redimensionamento contínuo (arrasto simulado): mede quanto tempo o preview fica desalinhado do layout.
        let (base_w, base_h) = (f64::from(1100) * dpr, f64::from(620) * dpr);
        let (mut samples, mut bad) = (0u32, 0u32);
        for i in 0..60 {
            let pw = (base_w + f64::from(i) * 6.0 * dpr) as u32;
            let ph = (base_h + f64::from(i) * 3.4 * dpr) as u32;
            let s0 = ctx.shared.seq();
            let _ = ctx.app_window.set_size(tauri::PhysicalSize::new(pw, ph));
            let t = Instant::now();
            let l = ctx
                .shared
                .wait_layout_after(s0, Duration::from_millis(40))
                .unwrap_or_else(|| lock(&ctx.shared.layout).clone());
            if let Ok(r) = ctx.set_geometry(&l, pat) {
                apply(r, false);
            }
            while t.elapsed() < Duration::from_millis(16) {
                if let Ok(p) = ctx.sample_at_fraction(&l, 0.08, 0.08) {
                    samples += 1;
                    if !near(p, GRAY) {
                        bad += 1;
                    }
                }
                thread::sleep(Duration::from_millis(2));
            }
        }
        let _ = ctx.app_window.set_size(tauri::PhysicalSize::new(
            (1280.0 * dpr) as u32,
            (720.0 * dpr) as u32,
        ));
        Ok(json!({
            "mode": mode, "dpr": dpr, "steps": steps,
            "continuous_resize": { "frames": 60, "samples": samples, "samples_without_pattern_at_expected_rect": bad,
                "bad_fraction": if samples > 0 { f64::from(bad) / f64::from(samples) } else { 0.0 } },
            "visual_flicker": "NÃO medido automaticamente — ver checklist manual em run.ps1",
        }))
    }

    fn fullscreen_phase(
        ctx: &Ctx,
        mode: &str,
        pat: (u32, u32),
        apply: &dyn Fn(Rect, bool),
    ) -> Result<Value, String> {
        ctx.app_window
            .set_fullscreen(true)
            .map_err(|e| format!("set_fullscreen(true): {e}"))?;
        thread::sleep(Duration::from_millis(900));
        let l = ctx.refresh_layout()?;
        let r = ctx.set_geometry(&l, pat)?;
        apply(r, false);
        thread::sleep(Duration::from_millis(500));
        let probe = ctx.sample_at_fraction(&l, 0.08, 0.08)?;
        let geo = (*lock(&ctx.geo)).ok_or("sem geometria")?;
        let id = ScreenSampler::new(1400, 200).and_then(|mut s| read_frame_id(&mut s, &geo));
        ctx.app_window
            .set_fullscreen(false)
            .map_err(|e| format!("set_fullscreen(false): {e}"))?;
        thread::sleep(Duration::from_millis(900));
        let l2 = ctx.refresh_layout()?;
        if let Ok(r2) = ctx.set_geometry(&l2, pat) {
            apply(r2, false);
        }
        Ok(json!({
            "mode": mode, "stage_physical_rect_fullscreen": [r.x, r.y, r.w, r.h], "client_size": win::client_size(ctx.hwnd).ok(),
            "probe_rgb": probe, "pattern_visible_in_fullscreen": near(probe, GRAY),
            "frame_id_decoded_from_screen": id.as_ref().ok().and_then(|v| *v), "frame_id_error": id.err(),
        }))
    }

    fn input_phase(ctx: &Ctx, mode: &str) -> Result<Value, String> {
        if ctx.cfg.skip_input {
            return Ok(json!({ "skipped": true }));
        }
        let focused = win::focus_window(ctx.hwnd);
        thread::sleep(Duration::from_millis(400));
        let l = ctx.refresh_layout()?;
        let (ox, oy) = ctx.screen_origin()?;
        let centre = |c: [f64; 4]| -> (i32, i32) {
            let r = phys(c, l.dpr);
            (ox + r.x + r.w / 2, oy + r.y + r.h / 2)
        };
        let r = ctx.stage_phys(&l);
        let bare = (
            ox + r.x + (f64::from(r.w) * 0.65) as i32,
            oy + r.y + (f64::from(r.h) * 0.5) as i32,
        );
        ctx.js("window.s1.setOverlay(true)");
        thread::sleep(Duration::from_millis(500));
        let l = ctx.refresh_layout()?;
        let (ob_x, ob_y) = centre(l.overlay_btn_css);
        let (tb_x, tb_y) = centre(l.toolbar_btn_css);
        let before = (
            ctx.shared.count("click_toolbar"),
            ctx.shared.count("click_overlay_btn"),
            ctx.shared.count("click_box"),
            win::NATIVE_LBUTTONDOWN.load(Ordering::Relaxed),
        );
        let accepted = (
            win::click_at(tb_x, tb_y)?,
            win::click_at(ob_x, ob_y)?,
            win::click_at(bare.0, bare.1)?,
        );
        thread::sleep(Duration::from_millis(400));
        let keys = win::key_press(0x41);
        thread::sleep(Duration::from_millis(300));
        ctx.js("window.s1.setOverlay(false)");
        let after = (
            ctx.shared.count("click_toolbar"),
            ctx.shared.count("click_overlay_btn"),
            ctx.shared.count("click_box"),
            win::NATIVE_LBUTTONDOWN.load(Ordering::Relaxed),
        );
        Ok(json!({
            "mode": mode, "foreground_set": focused, "sendinput_events_accepted": [accepted.0, accepted.1, accepted.2, keys],
            "html_toolbar_clicks_received": after.0 - before.0,
            "html_button_over_preview_clicks_received": after.1 - before.1,
            "html_clicks_on_bare_preview_received": after.2 - before.2,
            "native_hwnd_lbuttondown_received": after.3 - before.3,
            "html_keydown_received_total": ctx.shared.count("keydown"),
            "reading": "Esperado em P1 (nativo ABAIXO): HTML recebe todos os cliques e o HWND nativo não recebe nenhum. Se o botão sobre o preview não for recebido, há airspace. Em P1_above (controle): o nativo captura os cliques sobre o preview.",
        }))
    }

    // ---------------------------------------------------------------- modos

    fn mode_p1(ctx: &Ctx, above: bool) -> Value {
        let name = if above {
            "p1_above_control"
        } else {
            "p1_native_child_under_transparent_webview"
        };
        ctx.js("window.s1.setMode('p1')");
        let mut phases = Vec::new();
        let setup = (|| -> Result<(Layout, Rect), String> {
            let l = ctx.refresh_layout()?;
            let r = ctx.stage_phys(&l);
            win::place_child(ctx.child, r, above)?;
            ctx.set_geometry(&l, (r.w as u32, r.h as u32))?;
            Ok((l, r))
        })();
        let (_, r0) = match setup {
            Ok(v) => v,
            Err(e) => return json!({ "mode": name, "setup_error": e }),
        };
        let engine = Mutex::new(Some(P1Engine::start(
            ctx.gpu.clone(),
            ctx.pattern.clone(),
            ctx.surface.clone(),
            ctx.log.clone(),
            (r0.w as u32, r0.h as u32),
        )));
        let apply = |r: Rect, _hide: bool| {
            let _ = win::place_child(ctx.child, r, above);
            if let Some(e) = lock(&engine).as_ref() {
                e.set_size(r.w as u32, r.h as u32);
            }
            *lock(&ctx.geo) = lock(&ctx.geo).map(|g| Geometry {
                pat_w: r.w as u32,
                pat_h: r.h as u32,
                ..g
            });
        };
        thread::sleep(Duration::from_millis(800));
        phases.push(phase("probe_static", || probe_static(ctx, name)));
        if !above {
            let pat = (r0.w as u32, r0.h as u32);
            phases.push(phase("steady_cpu", || {
                steady(ctx, name, pat, &|| json!(null), false)
            }));
            ctx.log.clear();
            phases.push(phase("steady_latency", || {
                steady(ctx, name, pat, &|| json!(null), true)
            }));
            phases.push(phase("resize", || resize_phase(ctx, name, pat, &apply)));
            phases.push(phase("fullscreen", || {
                fullscreen_phase(ctx, name, pat, &apply)
            }));
        }
        phases.push(phase("input", || input_phase(ctx, name)));
        let engine_stats = lock(&engine).take().map_or(json!(null), P1Engine::stop);
        let _ = win::place_child(
            ctx.child,
            Rect {
                x: 0,
                y: 0,
                w: 1,
                h: 1,
            },
            false,
        );
        *lock(&ctx.geo) = None;
        json!({
            "mode": name, "child_above_webview": above, "phases": phases, "engine_stats_whole_mode": engine_stats,
            "copies": { "gpu_to_cpu": 0, "cpu_to_gpu": 0, "kind": "estrutural (não medido): swapchain DXGI no HWND do app, composto pelo DWM; sem cópia CPU" },
        })
    }

    fn mode_p2(ctx: &Ctx) -> Value {
        let name = "p2_sharedbuffer_to_webgl_canvas";
        let (w, h) = ctx.cfg.p2_res;
        let setup = (|| -> Result<(), String> {
            let mut mem = lock(&ctx.mem);
            if mem.is_none() {
                let size = (HEADER_BYTES + 1920 * 1080 * 4) as u64;
                *mem = Some(create_shared_buffer(
                    &ctx.app_window,
                    size,
                    &json!({ "header_bytes": HEADER_BYTES }).to_string(),
                )?);
            }
            Ok(())
        })();
        if let Err(e) = setup {
            return json!({ "mode": name, "setup_error": e });
        }
        ctx.js(&format!("window.s1.setMode('p2', {w}, {h})"));
        thread::sleep(Duration::from_millis(1200));
        let got = ctx.shared.count("p2_buffer_received");
        let Some(mem) = *lock(&ctx.mem) else {
            return json!({ "mode": name, "setup_error": "sem SharedBuffer" });
        };
        let engine = Mutex::new(Some(P2Engine::start(
            ctx.gpu.clone(),
            ctx.pattern.clone(),
            mem,
            ctx.log.clone(),
            (w, h),
            60.0,
        )));
        let pat = (w, h);
        let apply = |_r: Rect, _h: bool| {};
        let mut phases = Vec::new();
        thread::sleep(Duration::from_millis(800));
        phases
            .push(json!({ "name": "buffer_handshake", "page_received_sharedbuffer_events": got }));
        phases.push(phase("probe_static", || probe_static(ctx, name)));
        phases.push(phase("steady_cpu", || {
            steady(ctx, name, pat, &|| json!(null), false)
        }));
        ctx.log.clear();
        phases.push(phase("steady_latency", || {
            steady(ctx, name, pat, &|| json!(null), true)
        }));
        phases.push(phase("resize", || resize_phase(ctx, name, pat, &apply)));
        phases.push(phase("fullscreen", || {
            fullscreen_phase(ctx, name, pat, &apply)
        }));
        phases.push(phase("input", || input_phase(ctx, name)));
        let engine_stats = lock(&engine).take().map_or(json!(null), P2Engine::stop);
        ctx.js("window.s1.setMode('none')");
        *lock(&ctx.geo) = None;
        json!({
            "mode": name, "phases": phases, "engine_stats_whole_mode": engine_stats,
            "copies": { "gpu_to_cpu_readback": 1, "cpu_memcpy_into_shared_buffer": 1, "shared_buffer_to_browser_gpu_texture": ">=1 (interno ao navegador; tempo JS de upload+draw em page_stats)",
                "kind": "contagem estrutural; os tempos de readback/memcpy estão em engine_stats_whole_mode" },
        })
    }

    // ---------------------------------------------------------------- execução

    fn runtime_info(ctx: &Ctx) -> Value {
        let monitors: Vec<Value> = ctx
            .app_window
            .available_monitors()
            .map(|ms| ms.iter().map(|m| json!({ "name": m.name(), "size": [m.size().width, m.size().height], "scale_factor": m.scale_factor() })).collect())
            .unwrap_or_default();
        json!({
            "gpu_adapter": ctx.gpu.describe(),
            "window": { "scale_factor": ctx.app_window.scale_factor().ok(), "dpi_for_window": win::dpi_for_window(ctx.hwnd),
                "client_size": win::client_size(ctx.hwnd).ok(), "dwm_refresh_hz": win::dwm_refresh_hz() },
            "monitors": monitors, "page": lock(&ctx.shared.layout).raw.clone(),
            "logical_cores": thread::available_parallelism().map(std::num::NonZero::get).ok(),
        })
    }

    fn run_all(app: tauri::AppHandle, shared: Arc<Shared>, cfg: Cfg) -> Value {
        let started = utc_ms();
        let mut report = json!({
            "schema": "capia-s1-report/1", "harness_version": env!("CARGO_PKG_VERSION"), "label": cfg.label,
            "started_utc_ms": started, "config": { "modes": cfg.modes, "steady_secs": cfg.steady_secs, "p2_res": [cfg.p2_res.0, cfg.p2_res.1], "skip_input": cfg.skip_input },
            "notice": "Gerado por um harness que NÃO foi executado pelo autor (sem Windows). Se algum campo estiver ausente/erro, isso é dado, não falha silenciosa.",
            "modes": {},
        });
        let setup = (|| -> Result<Ctx, String> {
            let window = app
                .get_webview_window("main")
                .ok_or("janela 'main' não encontrada")?;
            if shared
                .wait_layout_after(0, Duration::from_secs(20))
                .is_none()
            {
                return Err("a página não reportou layout em 20 s (WebView2 não iniciou?)".into());
            }
            let hwnd = window.hwnd().map_err(|e| format!("hwnd: {e}"))?;
            let child = win::create_native_child(hwnd)?;
            win::place_child(
                child,
                Rect {
                    x: 0,
                    y: 0,
                    w: 1,
                    h: 1,
                },
                false,
            )?;
            let (gpu, surface) = Gpu::new(child.0 as isize)?;
            let gpu = Arc::new(gpu);
            let pattern = Arc::new(Mutex::new(Pattern::new(&gpu)));
            Ok(Ctx {
                app_window: window,
                hwnd,
                child,
                shared: shared.clone(),
                gpu,
                pattern,
                surface: Arc::new(surface),
                log: SubmitLog::new(Instant::now()),
                geo: Arc::new(Mutex::new(None)),
                cfg: cfg.clone(),
                mem: Mutex::new(None),
            })
        })();
        let ctx = match setup {
            Ok(c) => c,
            Err(e) => {
                report["setup_error"] = json!(e);
                return report;
            }
        };
        report["runtime"] = runtime_info(&ctx);
        let mut modes = Map::new();
        for m in &cfg.modes {
            let r = catch_unwind(AssertUnwindSafe(|| match m.as_str() {
                "p1" => mode_p1(&ctx, false),
                "p1_above" => mode_p1(&ctx, true),
                "p2" => mode_p2(&ctx),
                other => json!({ "error": format!("modo desconhecido: {other}") }),
            }))
            .unwrap_or_else(|_| json!({ "error": "panic no modo" }));
            modes.insert(m.clone(), r);
        }
        report["modes"] = Value::Object(modes);
        report["ended_utc_ms"] = json!(utc_ms());
        report["unavailable_metrics"] = json!([
            { "metric": "GPU utilization %", "why": "coletado por run.ps1 (contadores 'GPU Engine' do Windows), correlacionado pelas janelas started/ended_utc_ms de cada fase" },
            { "metric": "latência fóton-a-fóton", "why": "exige câmera de alta velocidade/fotodiodo; aqui só se mede 'submissão → aparece na tela pós-DWM' por leitura de pixels" },
            { "metric": "mudança de DPI/monitor", "why": "não automatizável com segurança; repetir run.ps1 em 100%/150%/200% e com 2 monitores (campo `label`)" },
            { "metric": "flicker/tearing visual no resize", "why": "só observável por humano; ver checklist manual" },
            { "metric": "cópias GPU/CPU como contadores de hardware", "why": "sem contador portátil; informado de forma estrutural em `copies`, com tempos de readback/memcpy medidos" },
            { "metric": "pacing interno do compositor do WebView2", "why": "não exposto; observado indiretamente via `visible_frame_interval_ms`" },
        ]);
        report
    }

    pub fn run() {
        let cfg = parse_args();
        let shared = Arc::new(Shared::default());
        let shared_for_setup = shared.clone();
        let cfg_for_setup = cfg.clone();
        tauri::Builder::default()
            .manage(shared.clone())
            .invoke_handler(tauri::generate_handler![s1_layout, s1_event, s1_page_stats])
            .setup(move |app| {
                let handle = app.handle().clone();
                let (shared, cfg) = (shared_for_setup.clone(), cfg_for_setup.clone());
                thread::spawn(move || {
                    let report = catch_unwind(AssertUnwindSafe(|| run_all(handle.clone(), shared, cfg.clone())))
                        .unwrap_or_else(|_| json!({ "schema": "capia-s1-report/1", "fatal": "panic no orquestrador" }));
                    match std::fs::write(&cfg.out, serde_json::to_string_pretty(&report).unwrap_or_default()) {
                        Ok(()) => eprintln!("[s1] RELATORIO {}", cfg.out),
                        Err(e) => eprintln!("[s1] falha ao gravar relatório: {e}"),
                    }
                    handle.exit(0);
                });
                Ok(())
            })
            .run(tauri::generate_context!())
            .expect("falha ao iniciar o harness S1");
    }
}
