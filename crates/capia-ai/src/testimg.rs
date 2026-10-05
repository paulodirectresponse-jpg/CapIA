//! Geradores determinísticos de mídia mínima para probes e testes (PNG sólido e WAV de silêncio).
//! Sem dependências: PNG com blocos `stored` do zlib, CRC-32 e Adler-32 próprios.

fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for &x in data {
        a = (a + u32::from(x)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let mut body = kind.to_vec();
    body.extend_from_slice(data);
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32(&body).to_be_bytes());
}

/// PNG RGB `w`×`h` de cor sólida.
pub fn png_solid(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((w as usize * 3 + 1) * h as usize);
    let row: Vec<u8> = core::iter::once(0u8) // filtro None
        .chain(core::iter::repeat_n(rgb, w as usize).flatten())
        .collect();
    for _ in 0..h {
        raw.extend_from_slice(&row);
    }
    // zlib: cabeçalho 0x78 0x01 + blocos stored (≤ 65535) + adler32
    let mut z = vec![0x78, 0x01];
    let mut chunks = raw.chunks(65_535).peekable();
    while let Some(c) = chunks.next() {
        z.push(u8::from(chunks.peek().is_none()));
        z.extend_from_slice(&(c.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(c.len() as u16)).to_le_bytes());
        z.extend_from_slice(c);
    }
    z.extend_from_slice(&adler32(&raw).to_be_bytes());
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&w.to_be_bytes());
    ihdr.extend_from_slice(&h.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    out
}

/// WAV PCM16 mono de silêncio (`ms` milissegundos a `rate` Hz).
pub fn wav_silence(ms: u32, rate: u32) -> Vec<u8> {
    let samples = (u64::from(rate) * u64::from(ms) / 1000) as u32;
    let data_len = samples * 2;
    let mut v = Vec::with_capacity(44 + data_len as usize);
    v.extend_from_slice(b"RIFF");
    v.extend_from_slice(&(36 + data_len).to_le_bytes());
    v.extend_from_slice(b"WAVEfmt ");
    v.extend_from_slice(&16u32.to_le_bytes());
    v.extend_from_slice(&1u16.to_le_bytes()); // PCM
    v.extend_from_slice(&1u16.to_le_bytes()); // mono
    v.extend_from_slice(&rate.to_le_bytes());
    v.extend_from_slice(&(rate * 2).to_le_bytes());
    v.extend_from_slice(&2u16.to_le_bytes());
    v.extend_from_slice(&16u16.to_le_bytes());
    v.extend_from_slice(b"data");
    v.extend_from_slice(&data_len.to_le_bytes());
    v.resize(44 + data_len as usize, 0);
    v
}

pub fn base64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::*;

    #[test]
    fn png_has_valid_signature_and_chunks() {
        let p = png_solid(4, 4, [255, 0, 0]);
        assert_eq!(&p[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert!(p.windows(4).any(|w| w == b"IHDR"));
        assert!(p.windows(4).any(|w| w == b"IEND"));
        // CRC conhecido do IEND vazio
        assert_eq!(&p[p.len() - 4..], &[0xAE, 0x42, 0x60, 0x82]);
    }

    #[test]
    fn wav_header_sizes() {
        let w = wav_silence(500, 16_000);
        assert_eq!(w.len(), 44 + 16_000);
        assert_eq!(&w[..4], b"RIFF");
    }
}
