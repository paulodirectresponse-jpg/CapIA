//! Helpers Win32 do harness S1 (somente Windows). Tudo aqui é medição/infraestrutura de teste.
#![allow(unsafe_code)]

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use windows::Win32::Foundation::{
    CloseHandle, FILETIME, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, ClientToScreen, CreateCompatibleDC,
    CreateDIBSection, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, HBITMAP, HDC, HGDIOBJ,
    ReleaseDC, SRCCOPY, SelectObject,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_KEYUP, MOUSEEVENTF_LEFTDOWN,
    MOUSEEVENTF_LEFTUP, MOUSEINPUT, SendInput, VIRTUAL_KEY,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, EnumChildWindows, GWL_EXSTYLE, GWL_STYLE, GetClassNameW,
    GetClientRect, GetWindowLongPtrW, GetWindowRect, HWND_BOTTOM, HWND_TOP, IsWindowVisible,
    RegisterClassExW, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetCursorPos, SetForegroundWindow,
    SetWindowPos, WM_LBUTTONDOWN, WM_MOUSEMOVE, WNDCLASSEXW, WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

/// Mensagens de mouse que chegaram ao HWND nativo (prova de quem recebe o input).
pub static NATIVE_LBUTTONDOWN: AtomicU64 = AtomicU64::new(0);
pub static NATIVE_MOUSEMOVE: AtomicU64 = AtomicU64::new(0);

unsafe extern "system" fn native_wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_LBUTTONDOWN => {
            NATIVE_LBUTTONDOWN.fetch_add(1, Ordering::Relaxed);
        }
        WM_MOUSEMOVE => {
            NATIVE_MOUSEMOVE.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
    unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
}

/// Cria o HWND filho nativo (alvo do swapchain wgpu em P1) sob a janela principal.
pub fn create_native_child(parent: HWND) -> Result<HWND, String> {
    unsafe {
        let hinstance = GetModuleHandleW(None).map_err(|e| format!("GetModuleHandleW: {e}"))?;
        let class = w!("CapIAS1Native");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(native_wndproc),
            hInstance: hinstance.into(),
            lpszClassName: class,
            ..Default::default()
        };
        // Registrar duas vezes devolve 0 sem ser fatal; o CreateWindowExW abaixo valida.
        let _ = RegisterClassExW(&wc);
        CreateWindowExW(
            Default::default(),
            class,
            PCWSTR::null(),
            WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS | WS_CLIPCHILDREN,
            0,
            0,
            320,
            180,
            Some(parent),
            None,
            Some(hinstance.into()),
            None,
        )
        .map_err(|e| format!("CreateWindowExW(child): {e}"))
    }
}

/// Posiciona o filho nativo (px físicos, relativos ao client da janela). `above` = acima do WebView2.
pub fn place_child(child: HWND, rect: Rect, above: bool) -> Result<(), String> {
    unsafe {
        SetWindowPos(
            child,
            Some(if above { HWND_TOP } else { HWND_BOTTOM }),
            rect.x,
            rect.y,
            rect.w.max(1),
            rect.h.max(1),
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        )
        .map_err(|e| format!("SetWindowPos: {e}"))
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

pub fn client_origin_on_screen(hwnd: HWND) -> Result<(i32, i32), String> {
    let mut p = POINT { x: 0, y: 0 };
    unsafe {
        if !ClientToScreen(hwnd, &mut p).as_bool() {
            return Err("ClientToScreen falhou".into());
        }
    }
    Ok((p.x, p.y))
}

pub fn client_size(hwnd: HWND) -> Result<(i32, i32), String> {
    let mut r = RECT::default();
    unsafe { GetClientRect(hwnd, &mut r).map_err(|e| format!("GetClientRect: {e}"))? };
    Ok((r.right - r.left, r.bottom - r.top))
}

pub fn dpi_for_window(hwnd: HWND) -> u32 {
    unsafe { GetDpiForWindow(hwnd) }
}

/// Período de refresh segundo o DWM (Hz) — `None` se indisponível.
pub fn dwm_refresh_hz() -> Option<f64> {
    unsafe {
        let mut info = DWM_TIMING_INFO {
            cbSize: std::mem::size_of::<DWM_TIMING_INFO>() as u32,
            ..Default::default()
        };
        DwmGetCompositionTimingInfo(HWND::default(), &mut info).ok()?;
        let r = info.rateRefresh;
        (r.uiDenominator != 0).then(|| f64::from(r.uiNumerator) / f64::from(r.uiDenominator))
    }
}

/// Lê pixels da TELA já composta pelo DWM (inclui WebView2 + filho nativo). Reutiliza DCs/bitmap.
pub struct ScreenSampler {
    screen: HDC,
    mem: HDC,
    bitmap: HBITMAP,
    bits: *mut u8,
    w: i32,
    h: i32,
}

// O sampler vive em uma única thread por vez.
unsafe impl Send for ScreenSampler {}

impl ScreenSampler {
    pub fn new(max_w: i32, max_h: i32) -> Result<Self, String> {
        unsafe {
            let screen = GetDC(None);
            if screen.0.is_null() {
                return Err("GetDC(NULL) falhou".into());
            }
            let mem = CreateCompatibleDC(Some(screen));
            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: max_w,
                    biHeight: -max_h, // top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let bitmap = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0)
                .map_err(|e| format!("CreateDIBSection: {e}"))?;
            SelectObject(mem, HGDIOBJ(bitmap.0));
            Ok(Self {
                screen,
                mem,
                bitmap,
                bits: bits.cast(),
                w: max_w,
                h: max_h,
            })
        }
    }

    /// Captura `w x h` a partir de (sx, sy) em coordenadas de tela. Retorna acesso BGRA via `pixel`.
    pub fn capture(&mut self, sx: i32, sy: i32, w: i32, h: i32) -> Result<(), String> {
        if w > self.w || h > self.h {
            return Err(format!(
                "captura {w}x{h} excede o buffer {}x{}",
                self.w, self.h
            ));
        }
        unsafe {
            BitBlt(
                self.mem,
                0,
                0,
                w,
                h,
                Some(self.screen),
                sx,
                sy,
                SRCCOPY | CAPTUREBLT,
            )
            .map_err(|e| format!("BitBlt: {e}"))
        }
    }

    /// RGB do pixel (x, y) da última captura (origem = canto da captura).
    pub fn pixel(&self, x: i32, y: i32) -> [u8; 3] {
        let idx = ((y * self.w + x) * 4) as usize;
        // SAFETY: x,y dentro de `capture`; o buffer é w*h*4 bytes.
        unsafe {
            let p = self.bits.add(idx);
            [*p.add(2), *p.add(1), *p]
        }
    }
}

impl Drop for ScreenSampler {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ(self.bitmap.0));
            let _ = DeleteDC(self.mem);
            ReleaseDC(None, self.screen);
        }
    }
}

/// Lê um conjunto de pontos da tela (conveniência para sondas estáticas).
pub fn sample_points(points: &[(i32, i32)]) -> Result<Vec<[u8; 3]>, String> {
    let mut s = ScreenSampler::new(4, 4)?;
    let mut out = Vec::with_capacity(points.len());
    for &(x, y) in points {
        s.capture(x, y, 1, 1)?;
        out.push(s.pixel(0, 0));
    }
    Ok(out)
}

fn filetime_100ns(ft: FILETIME) -> u64 {
    (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime)
}

/// Tempo de CPU (kernel+user, em 100 ns) por grupo: processo atual e descendentes (msedgewebview2 etc).
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuTimes {
    pub own_100ns: u64,
    pub webview_100ns: u64,
    pub other_children_100ns: u64,
    pub webview_process_count: u32,
}

pub fn cpu_times_for_tree() -> Result<CpuTimes, String> {
    unsafe {
        let me = GetCurrentProcessId();
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| format!("Toolhelp: {e}"))?;
        let mut procs: Vec<(u32, u32, String)> = Vec::new();
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        if Process32FirstW(snap, &mut entry).is_ok() {
            loop {
                let end = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..end]).to_lowercase();
                procs.push((entry.th32ProcessID, entry.th32ParentProcessID, name));
                if Process32NextW(snap, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        let parent: HashMap<u32, u32> = procs.iter().map(|p| (p.0, p.1)).collect();
        let is_desc = |mut pid: u32| {
            for _ in 0..16 {
                match parent.get(&pid) {
                    Some(&pp) if pp == me => return true,
                    Some(&pp) if pp != 0 && pp != pid => pid = pp,
                    _ => return false,
                }
            }
            false
        };
        let mut out = CpuTimes::default();
        for (pid, _, name) in &procs {
            let (own, desc) = (*pid == me, is_desc(*pid));
            if !own && !desc {
                continue;
            }
            let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, *pid) else {
                continue;
            };
            let (mut c, mut e, mut k, mut u) = (
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
                FILETIME::default(),
            );
            if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
                let t = filetime_100ns(k) + filetime_100ns(u);
                if own {
                    out.own_100ns += t;
                } else if name.contains("msedgewebview2") {
                    out.webview_100ns += t;
                    out.webview_process_count += 1;
                } else {
                    out.other_children_100ns += t;
                }
            }
            let _ = CloseHandle(h);
        }
        Ok(out)
    }
}

pub fn focus_window(hwnd: HWND) -> bool {
    unsafe { SetForegroundWindow(hwnd).as_bool() }
}

/// Clique esquerdo sintético no ponto de TELA (x, y). Retorna quantos eventos o SO aceitou.
pub fn click_at(x: i32, y: i32) -> Result<u32, String> {
    unsafe {
        SetCursorPos(x, y).map_err(|e| format!("SetCursorPos: {e}"))?;
        let mk = |flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        Ok(SendInput(
            &[mk(MOUSEEVENTF_LEFTDOWN), mk(MOUSEEVENTF_LEFTUP)],
            std::mem::size_of::<INPUT>() as i32,
        ))
    }
}

pub fn key_press(vk: u16) -> u32 {
    unsafe {
        let mk = |flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(vk),
                    wScan: 0,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        SendInput(
            &[mk(Default::default()), mk(KEYEVENTF_KEYUP)],
            std::mem::size_of::<INPUT>() as i32,
        )
    }
}

/// Descreve uma janela (classe, estilos, retangulo na tela, visibilidade) para o relatorio.
pub fn window_info(hwnd: HWND) -> serde_json::Value {
    unsafe {
        let mut name = [0u16; 128];
        let n = GetClassNameW(hwnd, &mut name);
        let class = String::from_utf16_lossy(&name[..n.max(0) as usize]);
        let mut r = RECT::default();
        let rect_ok = GetWindowRect(hwnd, &mut r).is_ok();
        serde_json::json!({
            "hwnd": hwnd.0 as isize,
            "class": class,
            "style": format!("{:#010x}", GetWindowLongPtrW(hwnd, GWL_STYLE) as u32),
            "ex_style": format!("{:#010x}", GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32),
            "visible": IsWindowVisible(hwnd).as_bool(),
            "screen_rect": if rect_ok { serde_json::json!([r.left, r.top, r.right - r.left, r.bottom - r.top]) } else { serde_json::Value::Null },
        })
    }
}

unsafe extern "system" fn collect_child(hwnd: HWND, lp: LPARAM) -> windows::core::BOOL {
    // SAFETY: `lp` aponta para o Vec vivo em `window_tree` durante a enumeracao.
    let out = unsafe { &mut *(lp.0 as *mut Vec<serde_json::Value>) };
    out.push(window_info(hwnd));
    true.into()
}

/// Janela principal + todos os descendentes (a ordem de enumeracao e a ordem Z, de cima para baixo).
pub fn window_tree(hwnd: HWND) -> Vec<serde_json::Value> {
    let mut out = vec![window_info(hwnd)];
    unsafe {
        let _ = EnumChildWindows(
            Some(hwnd),
            Some(collect_child),
            LPARAM(&mut out as *mut Vec<serde_json::Value> as isize),
        );
    }
    out
}
