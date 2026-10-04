//! Superfície de preview P2 (ADR-069): o render escreve o quadro RGBA numa região de memória
//! compartilhada com o WebView2 (`CreateSharedBuffer` + `PostSharedBufferToScript`) e o JS sobe a
//! textura WebGL **sem cópia por IPC**. Sem janela nativa irmã ⇒ sem airspace (P1 eliminado).
//!
//! Contrato do buffer (little-endian): cabeçalho de [`HEADER_BYTES`] bytes — `magic u32 = "CAPF"`,
//! `seq u32`, `width u32`, `height u32` — seguido de `width × height × 4` bytes RGBA8 (alpha reto).
//! O JS lê **somente depois** de a chamada de render responder e **antes** de pedir o próximo
//! quadro (o agendador do front mantém no máximo um pedido em voo), então não há leitura rasgada.

#[cfg(windows)]
mod win;
#[cfg(windows)]
pub use win::SharedSurface;

use std::fmt;

/// Tamanho reservado do cabeçalho (alinha o início dos pixels).
pub const HEADER_BYTES: usize = 64;
/// `"CAPF"` em little-endian.
pub const MAGIC: u32 = 0x4650_4143;

/// Falha ao criar/escrever a superfície (mensagem humana; o front cai no caminho por IPC).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceError(pub String);

impl fmt::Display for SurfaceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for SurfaceError {}

/// Região `(ptr, len)` onde os quadros são escritos. Fora do Windows (e nos testes) aponta para
/// memória comum; no Windows aponta para o SharedBuffer mapeado.
#[derive(Debug)]
pub struct FrameRegion {
    ptr: *mut u8,
    len: usize,
}

// SAFETY: a região é escrita por uma chamada de cada vez (o chamador serializa) e lida pelo JS só
// depois da resposta; o ponteiro em si é só um número.
#[allow(unsafe_code)]
unsafe impl Send for FrameRegion {}
#[allow(unsafe_code)]
unsafe impl Sync for FrameRegion {}

impl FrameRegion {
    /// # Safety
    /// `ptr` deve apontar para `len` bytes graváveis que vivam mais que esta região.
    #[allow(unsafe_code)]
    pub unsafe fn from_raw(ptr: *mut u8, len: usize) -> Self {
        Self { ptr, len }
    }

    pub fn capacity_pixels(&self) -> usize {
        self.len.saturating_sub(HEADER_BYTES) / 4
    }

    /// Escreve cabeçalho + pixels. Erro se o quadro não cabe ou `rgba` não tem `w*h*4` bytes.
    #[allow(unsafe_code)]
    pub fn write_frame(
        &self,
        seq: u32,
        width: u32,
        height: u32,
        rgba: &[u8],
    ) -> Result<(), SurfaceError> {
        let need = (width as usize)
            .checked_mul(height as usize)
            .and_then(|p| p.checked_mul(4))
            .ok_or_else(|| SurfaceError("frame size overflow".into()))?;
        if rgba.len() != need {
            return Err(SurfaceError(format!(
                "frame has {} bytes, expected {need} for {width}x{height}",
                rgba.len()
            )));
        }
        if HEADER_BYTES + need > self.len {
            return Err(SurfaceError(format!(
                "frame {width}x{height} does not fit the shared buffer ({} bytes)",
                self.len
            )));
        }
        let mut header = [0u8; HEADER_BYTES];
        header[0..4].copy_from_slice(&MAGIC.to_le_bytes());
        header[4..8].copy_from_slice(&seq.to_le_bytes());
        header[8..12].copy_from_slice(&width.to_le_bytes());
        header[12..16].copy_from_slice(&height.to_le_bytes());
        // SAFETY: `ptr..ptr+len` é gravável (contrato de `from_raw`) e o tamanho foi conferido acima.
        unsafe {
            // pixels primeiro, cabeçalho por último: quem confere o `seq` nunca vê quadro pela metade
            std::ptr::copy_nonoverlapping(rgba.as_ptr(), self.ptr.add(HEADER_BYTES), need);
            std::ptr::copy_nonoverlapping(header.as_ptr(), self.ptr, HEADER_BYTES);
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn region(buf: &mut Vec<u8>) -> FrameRegion {
        // SAFETY: `buf` vive durante todo o teste.
        #[allow(unsafe_code)]
        unsafe {
            FrameRegion::from_raw(buf.as_mut_ptr(), buf.len())
        }
    }

    #[test]
    fn writes_header_then_pixels() {
        let mut buf = vec![0u8; HEADER_BYTES + 2 * 2 * 4];
        let r = region(&mut buf);
        let px: Vec<u8> = (0..16).collect();
        r.write_frame(7, 2, 2, &px).unwrap();
        assert_eq!(&buf[0..4], b"CAPF");
        assert_eq!(u32::from_le_bytes(buf[4..8].try_into().unwrap()), 7);
        assert_eq!(u32::from_le_bytes(buf[8..12].try_into().unwrap()), 2);
        assert_eq!(u32::from_le_bytes(buf[12..16].try_into().unwrap()), 2);
        assert_eq!(&buf[HEADER_BYTES..], &px[..]);
    }

    #[test]
    fn refuses_frames_that_do_not_fit_or_have_wrong_size() {
        let mut buf = vec![0u8; HEADER_BYTES + 8];
        let r = region(&mut buf);
        assert!(r.write_frame(1, 2, 2, &[0; 16]).is_err()); // não cabe
        assert!(r.write_frame(1, 1, 2, &[0; 7]).is_err()); // tamanho errado
        assert!(r.write_frame(1, 1, 2, &[1; 8]).is_ok());
        assert_eq!(r.capacity_pixels(), 2);
        assert_eq!(buf[0..4], MAGIC.to_le_bytes());
    }

    #[test]
    fn never_writes_past_the_end() {
        let mut buf = vec![0xEEu8; HEADER_BYTES + 4 + 16];
        let r = {
            // região menor que o vetor: os 16 bytes finais são "sentinela"
            #[allow(unsafe_code)]
            unsafe {
                FrameRegion::from_raw(buf.as_mut_ptr(), HEADER_BYTES + 4)
            }
        };
        r.write_frame(1, 1, 1, &[9; 4]).unwrap();
        assert!(buf[HEADER_BYTES + 4..].iter().all(|b| *b == 0xEE));
    }
}
