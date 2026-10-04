//! Motores de render (P1/P2) e amostradores de medição do harness S1.
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::gpu::{
    BLOCK_PX, CODE_BLOCKS, Gpu, HEADER_BYTES, Pattern, SubmitLog, align256, percentiles,
};
use crate::win::{self, ScreenSampler};

type SharedPattern = Arc<Mutex<Pattern>>;

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ------------------------------------------------------------------------------------------ P1

/// P1: swapchain wgpu (Fifo) no HWND nativo filho.
pub struct P1Engine {
    stop: Arc<AtomicBool>,
    size: Arc<(AtomicU32, AtomicU32)>,
    handle: Option<JoinHandle<Value>>,
}

impl P1Engine {
    pub fn start(
        gpu: Arc<Gpu>,
        pattern: SharedPattern,
        surface: Arc<wgpu::Surface<'static>>,
        log: Arc<SubmitLog>,
        size: (u32, u32),
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let sz = Arc::new((AtomicU32::new(size.0), AtomicU32::new(size.1)));
        let (stop2, sz2) = (stop.clone(), sz.clone());
        let handle = thread::spawn(move || p1_loop(&gpu, &pattern, &surface, &log, &stop2, &sz2));
        Self {
            stop,
            size: sz,
            handle: Some(handle),
        }
    }

    pub fn set_size(&self, w: u32, h: u32) {
        self.size.0.store(w.max(16), Ordering::Relaxed);
        self.size.1.store(h.max(16), Ordering::Relaxed);
    }

    pub fn stop(mut self) -> Value {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .map_or(json!({ "error": "no thread" }), |h| {
                h.join()
                    .unwrap_or_else(|_| json!({ "error": "p1 thread panicked" }))
            })
    }
}

fn p1_loop(
    gpu: &Gpu,
    pattern: &SharedPattern,
    surface: &wgpu::Surface<'static>,
    log: &SubmitLog,
    stop: &AtomicBool,
    size: &(AtomicU32, AtomicU32),
) -> Value {
    let caps = surface.get_capabilities(&gpu.adapter);
    let format = caps
        .formats
        .iter()
        .copied()
        .find(|f| *f == wgpu::TextureFormat::Bgra8Unorm)
        .or_else(|| caps.formats.iter().copied().find(|f| !f.is_srgb()))
        .or_else(|| caps.formats.first().copied());
    let Some(format) = format else {
        return json!({ "error": "surface sem formatos" });
    };
    let alpha = caps
        .alpha_modes
        .first()
        .copied()
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);
    let configure = |w: u32, h: u32| {
        surface.configure(
            &gpu.device,
            &wgpu::SurfaceConfiguration {
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                format,
                width: w,
                height: h,
                present_mode: wgpu::PresentMode::Fifo,
                desired_maximum_frame_latency: 2,
                alpha_mode: alpha,
                view_formats: vec![],
            },
        );
    };
    let mut cur = (
        size.0.load(Ordering::Relaxed),
        size.1.load(Ordering::Relaxed),
    );
    configure(cur.0, cur.1);
    let (mut frames, mut errors, mut reconfigs, mut frame_no) = (0u64, 0u64, 0u64, 0u32);
    let mut intervals_ms: Vec<f64> = Vec::new();
    let mut acquire_ms: Vec<f64> = Vec::new();
    let mut last_return: Option<Instant> = None;
    let begin = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let want = (
            size.0.load(Ordering::Relaxed),
            size.1.load(Ordering::Relaxed),
        );
        if want != cur {
            cur = want;
            configure(cur.0, cur.1);
            reconfigs += 1;
        }
        let a0 = Instant::now();
        let tex = match surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t)
            | wgpu::CurrentSurfaceTexture::Suboptimal(t) => t,
            wgpu::CurrentSurfaceTexture::Outdated => {
                configure(cur.0, cur.1);
                reconfigs += 1;
                continue;
            }
            _ => {
                errors += 1;
                thread::sleep(Duration::from_millis(2));
                continue;
            }
        };
        let got = Instant::now();
        acquire_ms.push((got - a0).as_secs_f64() * 1e3);
        if let Some(prev) = last_return {
            intervals_ms.push((got - prev).as_secs_f64() * 1e3);
        }
        last_return = Some(got);
        let view = tex
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("s1-p1"),
            });
        lock(pattern).encode(
            gpu,
            &mut enc,
            &view,
            format,
            frame_no,
            cur,
            begin.elapsed().as_secs_f32(),
        );
        log.record(frame_no);
        gpu.queue.submit([enc.finish()]);
        tex.present();
        frames += 1;
        frame_no = frame_no.wrapping_add(1);
    }
    let secs = begin.elapsed().as_secs_f64();
    let refresh = win::dwm_refresh_hz();
    let period = refresh.map_or(1000.0 / 60.0, |hz| 1000.0 / hz);
    let missed = intervals_ms.iter().filter(|&&i| i > period * 1.5).count();
    json!({
        "engine": "p1_native_child_swapchain",
        "surface_format": format!("{format:?}"), "alpha_mode": format!("{alpha:?}"), "present_mode": "Fifo",
        "frames_presented": frames, "seconds": secs, "avg_fps": frames as f64 / secs.max(1e-9),
        "dwm_refresh_hz": refresh, "assumed_period_ms": period,
        "present_interval_ms": percentiles(&mut intervals_ms),
        "intervals_over_1_5x_period": missed,
        "acquire_ms": percentiles(&mut acquire_ms),
        "surface_errors": errors, "reconfigures": reconfigs,
    })
}

// ------------------------------------------------------------------------------------------ P2

/// Região de memória compartilhada com o WebView2 (SharedBuffer). Escrita pelo render, lida pelo JS.
#[derive(Clone, Copy)]
pub struct SharedMem {
    pub ptr: *mut u8,
    pub len: usize,
}
unsafe impl Send for SharedMem {}
unsafe impl Sync for SharedMem {}

pub struct P2Engine {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Value>>,
}

impl P2Engine {
    pub fn start(
        gpu: Arc<Gpu>,
        pattern: SharedPattern,
        mem: SharedMem,
        log: Arc<SubmitLog>,
        res: (u32, u32),
        fps: f64,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || p2_loop(&gpu, &pattern, mem, &log, res, fps, &stop2));
        Self {
            stop,
            handle: Some(handle),
        }
    }

    pub fn stop(mut self) -> Value {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .map_or(json!({ "error": "no thread" }), |h| {
                h.join()
                    .unwrap_or_else(|_| json!({ "error": "p2 thread panicked" }))
            })
    }
}

#[allow(unsafe_code)]
fn p2_loop(
    gpu: &Gpu,
    pattern: &SharedPattern,
    mem: SharedMem,
    log: &SubmitLog,
    res: (u32, u32),
    fps: f64,
    stop: &AtomicBool,
) -> Value {
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (w, h) = res;
    let need = HEADER_BYTES + (w * h * 4) as usize;
    if mem.len < need {
        return json!({ "error": format!("SharedBuffer pequeno: {} < {}", mem.len, need) });
    }
    let tex = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("s1-p2"),
        size: wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    let bpr = align256(w * 4);
    let readback = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("s1-p2-readback"),
        size: u64::from(bpr) * u64::from(h),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    // Cabeçalho: w, h (u32 LE). O contador de frame (offset 0) só é escrito DEPOIS dos pixels.
    unsafe {
        std::ptr::copy_nonoverlapping(w.to_le_bytes().as_ptr(), mem.ptr.add(4), 4);
        std::ptr::copy_nonoverlapping(h.to_le_bytes().as_ptr(), mem.ptr.add(8), 4);
    }
    let period = Duration::from_secs_f64(1.0 / fps);
    let (mut render_ms, mut readback_ms, mut copy_ms, mut total_ms): (
        Vec<f64>,
        Vec<f64>,
        Vec<f64>,
        Vec<f64>,
    ) = (vec![], vec![], vec![], vec![]);
    let (mut produced, mut late, mut frame_no) = (0u64, 0u64, 1u32);
    let mut readback_timeouts = 0u32;
    let begin = Instant::now();
    let mut next = begin;
    while !stop.load(Ordering::Relaxed) {
        let f0 = Instant::now();
        let mut enc = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("s1-p2"),
            });
        lock(pattern).encode(
            gpu,
            &mut enc,
            &view,
            format,
            frame_no,
            res,
            begin.elapsed().as_secs_f32(),
        );
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bpr),
                    rows_per_image: Some(h),
                },
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit([enc.finish()]);
        let f1 = Instant::now();
        let slice = readback.slice(..);
        let mapped = Arc::new(AtomicBool::new(false));
        let mapped2 = mapped.clone();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            if r.is_ok() {
                mapped2.store(true, Ordering::Release);
            }
        });
        // espera limitada: um device perdido/travado nao pode pendurar o harness inteiro
        let _ = gpu.device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: Some(Duration::from_secs(5)),
        });
        if !mapped.load(Ordering::Acquire) {
            readback_timeouts += 1;
            if readback_timeouts >= 1 {
                return json!({ "error": "readback GPU->CPU nao completou (timeout de 5 s): device travado/perdido?",
                    "frames_produced": produced, "readback_timeouts": readback_timeouts });
            }
            continue;
        }
        let f2 = Instant::now();
        {
            let data = slice.get_mapped_range();
            let row = (w * 4) as usize;
            for y in 0..h as usize {
                let src = &data[y * bpr as usize..y * bpr as usize + row];
                // SAFETY: `need` foi validado contra `mem.len`; as linhas não se sobrepõem.
                unsafe {
                    std::ptr::copy_nonoverlapping(
                        src.as_ptr(),
                        mem.ptr.add(HEADER_BYTES + y * row),
                        row,
                    )
                };
            }
        }
        readback.unmap();
        let f3 = Instant::now();
        std::sync::atomic::fence(Ordering::Release);
        // SAFETY: offset 0 está dentro do cabeçalho; o JS só lê (plain read, x86 coerente).
        unsafe { std::ptr::copy_nonoverlapping(frame_no.to_le_bytes().as_ptr(), mem.ptr, 4) };
        log.record(frame_no);
        render_ms.push((f1 - f0).as_secs_f64() * 1e3);
        readback_ms.push((f2 - f1).as_secs_f64() * 1e3);
        copy_ms.push((f3 - f2).as_secs_f64() * 1e3);
        total_ms.push((f3 - f0).as_secs_f64() * 1e3);
        produced += 1;
        frame_no = frame_no.wrapping_add(1).max(1);
        next += period;
        let now = Instant::now();
        if now > next + period {
            late += 1;
            next = now;
        } else if next > now {
            thread::sleep(next - now);
        }
    }
    let secs = begin.elapsed().as_secs_f64();
    json!({
        "engine": "p2_offscreen_readback_sharedbuffer", "resolution": [w, h], "target_fps": fps,
        "frames_produced": produced, "seconds": secs, "avg_fps": produced as f64 / secs.max(1e-9),
        "frames_that_missed_their_slot": late, "readback_timeouts": readback_timeouts,
        "render_and_submit_ms": percentiles(&mut render_ms),
        "gpu_to_cpu_readback_wait_ms": percentiles(&mut readback_ms),
        "memcpy_into_sharedbuffer_ms": percentiles(&mut copy_ms),
        "produce_total_ms": percentiles(&mut total_ms),
        "bytes_per_frame": w * h * 4,
    })
}

// ------------------------------------------------------------------------------------ amostrador

/// Retângulo do padrão na tela (px físicos) e a resolução do padrão (para mapear blocos).
#[derive(Clone, Copy, Debug)]
pub struct Geometry {
    pub screen_x: i32,
    pub screen_y: i32,
    pub w: i32,
    pub h: i32,
    pub pat_w: u32,
    pub pat_h: u32,
}

fn luma(p: [u8; 3]) -> i32 {
    (i32::from(p[0]) * 299 + i32::from(p[1]) * 587 + i32::from(p[2]) * 114) / 1000
}

/// Captura a faixa de codigo da TELA e devolve o luma dos 18 blocos (calibracao + 16 bits + calibracao).
pub fn read_strip(s: &mut ScreenSampler, g: &Geometry) -> Result<Vec<i32>, String> {
    let sx = f64::from(g.w) / f64::from(g.pat_w);
    let sy = f64::from(g.h) / f64::from(g.pat_h);
    let ch = (f64::from(BLOCK_PX) * sy).ceil() as i32 + 2;
    let cw = (f64::from(BLOCK_PX * CODE_BLOCKS) * sx).ceil() as i32 + 2;
    let top = g.screen_y + g.h - (f64::from(BLOCK_PX) * sy).ceil() as i32 - 1;
    s.capture(g.screen_x, top, cw, ch)?;
    let cy = ((f64::from(BLOCK_PX) * sy) / 2.0) as i32 + 1;
    Ok((0..CODE_BLOCKS)
        .map(|i| luma(s.pixel(((f64::from(i * BLOCK_PX + BLOCK_PX / 2)) * sx) as i32, cy)))
        .collect())
}

/// Decodifica o numero do frame a partir do luma dos blocos (None se a calibracao nao e valida).
pub fn decode_strip(v: &[i32]) -> Option<u16> {
    let (white, black) = (*v.first()?, *v.get(CODE_BLOCKS as usize - 1)?);
    if white - black < 100 {
        return None;
    }
    let thr = (white + black) / 2;
    let mut out = 0u16;
    for bit in 0..16usize {
        if v[bit + 1] > thr {
            out |= 1 << bit;
        }
    }
    Some(out)
}

/// Captura a faixa de codigo da TELA e decodifica o numero do frame (None se a faixa nao e valida).
pub fn read_frame_id(s: &mut ScreenSampler, g: &Geometry) -> Result<Option<u16>, String> {
    Ok(decode_strip(&read_strip(s, g)?))
}

/// Amostrador contínuo: mede em que instante cada frame submetido aparece NA TELA (pós-DWM).
pub struct LatencySampler {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Value>>,
}

impl LatencySampler {
    pub fn start(geo: Arc<Mutex<Option<Geometry>>>, log: Arc<SubmitLog>) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || sampler_loop(&geo, &log, &stop2));
        Self {
            stop,
            handle: Some(handle),
        }
    }
    pub fn stop(mut self) -> Value {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .map_or(json!({ "error": "no thread" }), |h| {
                h.join()
                    .unwrap_or_else(|_| json!({ "error": "sampler panicked" }))
            })
    }
}

fn sampler_loop(geo: &Mutex<Option<Geometry>>, log: &SubmitLog, stop: &AtomicBool) -> Value {
    let mut sampler = match ScreenSampler::new(1400, 200) {
        Ok(s) => s,
        Err(e) => return json!({ "error": e }),
    };
    let (mut samples, mut invalid, mut errors, mut skipped, mut observed) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut latencies, mut distinct_intervals): (Vec<f64>, Vec<f64>) = (vec![], vec![]);
    let (mut last_id, mut last_seen_us): (Option<u16>, Option<u64>) = (None, None);
    let mut first_error: Option<String> = None;
    let mut first_invalid: Vec<Value> = Vec::new();
    let begin = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        let Some(g) = *lock(geo) else {
            thread::sleep(Duration::from_millis(1));
            continue;
        };
        match read_strip(&mut sampler, &g).map(|v| (decode_strip(&v), v)) {
            Ok((Some(id), _)) => {
                samples += 1;
                let now_us = log.base.elapsed().as_micros() as u64;
                if last_id != Some(id) {
                    if let Some(prev) = last_id {
                        skipped += u64::from(id.wrapping_sub(prev).saturating_sub(1)).min(1000);
                    }
                    if let Some(prev_us) = last_seen_us {
                        distinct_intervals.push((now_us - prev_us) as f64 / 1e3);
                    }
                    if let Some(sub_us) = log.get(id) {
                        latencies.push(now_us.saturating_sub(sub_us) as f64 / 1e3);
                    }
                    observed += 1;
                    last_id = Some(id);
                    last_seen_us = Some(now_us);
                }
            }
            Ok((None, strip)) => {
                invalid += 1;
                if first_invalid.len() < 3 {
                    first_invalid.push(json!({ "strip_luma": strip, "geometry": format!("{g:?}") }));
                }
            }
            Err(e) => {
                errors += 1;
                first_error.get_or_insert(e);
            }
        }
    }
    let secs = begin.elapsed().as_secs_f64();
    json!({
        "method": "BitBlt da tela (pós-DWM) lendo faixa de 16 bits; latência = instante em que o frame aparece menos instante de submissão",
        "caveats": "não é latência fóton-a-fóton; resolução limitada pela taxa de captura; inclui jitter de escalonamento da thread",
        "capture_rate_hz": samples as f64 / secs.max(1e-9), "valid_samples": samples, "invalid_samples": invalid,
        "capture_errors": errors, "first_capture_error": first_error, "first_invalid_samples": first_invalid,
        "distinct_frames_observed": observed, "frame_ids_skipped_between_observations": skipped,
        "submit_to_visible_ms": percentiles(&mut latencies),
        "visible_frame_interval_ms": percentiles(&mut distinct_intervals),
    })
}

// ------------------------------------------------------------------------------------ recursos

pub struct ResourceSampler {
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<Value>>,
}

impl ResourceSampler {
    pub fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let stop2 = stop.clone();
        let handle = thread::spawn(move || resource_loop(&stop2));
        Self {
            stop,
            handle: Some(handle),
        }
    }
    pub fn stop(mut self) -> Value {
        self.stop.store(true, Ordering::Relaxed);
        self.handle
            .take()
            .map_or(json!({ "error": "no thread" }), |h| {
                h.join()
                    .unwrap_or_else(|_| json!({ "error": "resource sampler panicked" }))
            })
    }
}

fn resource_loop(stop: &AtomicBool) -> Value {
    let cores = thread::available_parallelism().map_or(1, std::num::NonZero::get) as f64;
    let first = match win::cpu_times_for_tree() {
        Ok(t) => t,
        Err(e) => return json!({ "error": e }),
    };
    let t0 = Instant::now();
    let mut prev = (first, t0);
    let (mut own, mut web, mut other): (Vec<f64>, Vec<f64>, Vec<f64>) = (vec![], vec![], vec![]);
    let mut wv_procs = 0;
    while !stop.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(500));
        let Ok(now) = win::cpu_times_for_tree() else {
            continue;
        };
        let t = Instant::now();
        let dt = (t - prev.1).as_secs_f64().max(1e-6) * 1e7; // em unidades de 100 ns
        own.push(now.own_100ns.saturating_sub(prev.0.own_100ns) as f64 / dt);
        web.push(now.webview_100ns.saturating_sub(prev.0.webview_100ns) as f64 / dt);
        other.push(
            now.other_children_100ns
                .saturating_sub(prev.0.other_children_100ns) as f64
                / dt,
        );
        wv_procs = now.webview_process_count;
        prev = (now, t);
    }
    let mean = |v: &Vec<f64>| {
        if v.is_empty() {
            0.0
        } else {
            v.iter().sum::<f64>() / v.len() as f64
        }
    };
    let (o, w, x) = (mean(&own), mean(&web), mean(&other));
    json!({
        "logical_cores": cores, "samples": own.len(), "webview2_process_count": wv_procs,
        "cores_used": { "app_process": o, "webview2_processes": w, "other_children": x, "total": o + w + x },
        "percent_of_machine": { "app_process": o / cores * 100.0, "webview2_processes": w / cores * 100.0, "total": (o + w + x) / cores * 100.0 },
        "note": "CPU via GetProcessTimes (processo do app + descendentes). GPU % NÃO é obtido aqui: run.ps1 coleta os contadores 'GPU Engine' do Windows por fase.",
    })
}
