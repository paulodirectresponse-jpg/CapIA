//! Borda COM do WebView2 (Windows). `unsafe` só aqui: criar o SharedBuffer, obter o ponteiro e
//! postá-lo ao script da página. Mesma sequência medida no S1 (`tools/s1-preview-spike`).
#![allow(unsafe_code)]

use crate::{FrameRegion, SurfaceError};
use std::sync::mpsc;
use std::time::Duration;
use tauri::WebviewWindow;
use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY, ICoreWebView2_17, ICoreWebView2Environment12,
    ICoreWebView2SharedBuffer,
};
use windows::core::{HSTRING, Interface, PCWSTR};

/// SharedBuffer vivo + a região mapeada. O COM é mantido enquanto a superfície existir.
#[derive(Debug)]
pub struct SharedSurface {
    region: FrameRegion,
    // mantém o objeto COM vivo (o JS usa a memória enquanto ele existir)
    _keep: KeepAlive,
}

struct KeepAlive(#[allow(dead_code)] ICoreWebView2SharedBuffer);

impl std::fmt::Debug for KeepAlive {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KeepAlive(ICoreWebView2SharedBuffer)")
    }
}

// SAFETY: o objeto COM é criado na thread de UI, mas só é guardado (nunca chamado) por outras
// threads; a memória mapeada é acessada via `FrameRegion` (já Send/Sync por contrato).
unsafe impl Send for KeepAlive {}
unsafe impl Sync for KeepAlive {}

impl SharedSurface {
    /// Cria um SharedBuffer de `size` bytes e o posta ao script da página (`sharedbufferreceived`).
    /// A closure do `with_webview` roda na thread de UI; esperamos até 30 s por ela.
    pub fn create(
        window: &WebviewWindow,
        size: usize,
        meta_json: &str,
    ) -> Result<Self, SurfaceError> {
        let (tx, rx) = mpsc::channel();
        let meta = HSTRING::from(meta_json);
        window
            .with_webview(move |wv| {
                let result = (|| -> Result<SharedSurface, SurfaceError> {
                    let err =
                        |what: &str, e: windows::core::Error| SurfaceError(format!("{what}: {e}"));
                    let env: ICoreWebView2Environment12 = wv
                        .environment()
                        .cast()
                        .map_err(|e| err("Environment12 indisponível (WebView2 antigo?)", e))?;
                    let buf = unsafe { env.CreateSharedBuffer(size as u64) }
                        .map_err(|e| err("CreateSharedBuffer", e))?;
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    unsafe { buf.Buffer(&mut ptr) }.map_err(|e| err("SharedBuffer::Buffer", e))?;
                    if ptr.is_null() {
                        return Err(SurfaceError("SharedBuffer sem memória mapeada".into()));
                    }
                    let core = unsafe { wv.controller().CoreWebView2() }
                        .map_err(|e| err("CoreWebView2", e))?;
                    let wv17: ICoreWebView2_17 = core
                        .cast()
                        .map_err(|e| err("ICoreWebView2_17 indisponível", e))?;
                    unsafe {
                        wv17.PostSharedBufferToScript(
                            &buf,
                            COREWEBVIEW2_SHARED_BUFFER_ACCESS_READ_ONLY,
                            PCWSTR(meta.as_ptr()),
                        )
                    }
                    .map_err(|e| err("PostSharedBufferToScript", e))?;
                    Ok(SharedSurface {
                        region: unsafe { FrameRegion::from_raw(ptr, size) },
                        _keep: KeepAlive(buf),
                    })
                })();
                let _ = tx.send(result);
            })
            .map_err(|e| SurfaceError(format!("with_webview: {e}")))?;
        rx.recv_timeout(Duration::from_secs(30)).map_err(|e| {
            SurfaceError(format!(
                "timeout criando o SharedBuffer: a closure de with_webview não rodou na thread de UI ({e})"
            ))
        })?
    }

    pub fn region(&self) -> &FrameRegion {
        &self.region
    }
}
