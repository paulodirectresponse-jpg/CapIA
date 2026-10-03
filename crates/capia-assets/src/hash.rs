//! Hash de conteúdo em *streaming* (ADR-046 §2): SHA-256 em blocos de 1 MiB; o arquivo **nunca**
//! é carregado inteiro na memória (teste mede o pico de leitura).

use crate::error::{AssetError, AssetErrorCode};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;

const BLOCK: usize = 1024 * 1024;

/// `sha256:<64 hex minúsculos>`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    pub const PREFIX: &'static str = "sha256:";

    /// Valida o formato (um hash vindo de arquivo/CLI é entrada externa).
    pub fn parse(s: &str) -> Option<Self> {
        let hex = s.strip_prefix(Self::PREFIX)?;
        (hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
            .then(|| Self(s.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn hex(&self) -> &str {
        &self.0[Self::PREFIX.len()..]
    }
}

impl core::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileDigest {
    pub hash: ContentHash,
    pub size: u64,
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Hash de qualquer leitor, em blocos de 1 MiB.
pub fn hash_reader<R: Read>(mut r: R) -> std::io::Result<FileDigest> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; BLOCK];
    let mut size: u64 = 0;
    loop {
        let n = match r.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        h.update(&buf[..n]);
        size = size.saturating_add(n as u64);
    }
    Ok(FileDigest {
        hash: ContentHash(format!("{}{}", ContentHash::PREFIX, hex(&h.finalize()))),
        size,
    })
}

/// Abre sem bloquear: um FIFO trocado no lugar do arquivo (corrida após a checagem de tipo) não
/// pode travar o import — o `is_file()` do handle logo abaixo o rejeita.
#[cfg(unix)]
fn open_nonblocking(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    #[cfg(target_os = "linux")]
    const O_NONBLOCK: i32 = 0o4000;
    #[cfg(not(target_os = "linux"))]
    const O_NONBLOCK: i32 = 0x0004;
    std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(O_NONBLOCK)
        .open(path)
}

#[cfg(not(unix))]
fn open_nonblocking(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::File::open(path)
}

/// Carimbo barato do arquivo (tamanho + mtime em ns) para detectar mudança durante o processamento.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileStamp {
    pub size: u64,
    pub modified_ns: Option<i128>,
}

impl FileStamp {
    fn of(m: &std::fs::Metadata) -> Self {
        let modified_ns =
            m.modified()
                .ok()
                .map(|t| match t.duration_since(std::time::UNIX_EPOCH) {
                    Ok(d) => d.as_nanos() as i128,
                    Err(e) => -(e.duration().as_nanos() as i128),
                });
        Self {
            size: m.len(),
            modified_ns,
        }
    }
}

/// Carimbo do arquivo no caminho (segue symlinks).
pub fn file_stamp(path: &Path) -> Result<FileStamp, AssetError> {
    std::fs::metadata(path)
        .map(|m| FileStamp::of(&m))
        .map_err(|e| {
            AssetError::new(
                AssetErrorCode::AssetIo,
                format!("cannot stat `{}`: {e}", path.display()),
            )
        })
}

fn hash_file_inner(
    path: &Path,
    change_code: AssetErrorCode,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<FileDigest, AssetError> {
    let io = |e: std::io::Error| {
        AssetError::new(
            AssetErrorCode::AssetIo,
            format!("cannot read `{}`: {e}", path.display()),
        )
    };
    let changed = || {
        AssetError::new(
            change_code,
            format!("`{}` changed while it was being read", path.display()),
        )
    };
    let mut f = open_nonblocking(path).map_err(io)?;
    let before = f.metadata().map_err(io)?;
    if !before.is_file() {
        return Err(AssetError::new(
            AssetErrorCode::AssetNotRegularFile,
            format!("`{}` is not a regular file", path.display()),
        ));
    }
    let stamp = FileStamp::of(&before);
    let mut h = Sha256::new();
    let mut buf = vec![0u8; BLOCK];
    let mut size: u64 = 0;
    loop {
        if cancel() {
            return Err(AssetError::new(
                AssetErrorCode::AssetCancelled,
                "hashing was cancelled",
            ));
        }
        let n = match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(io(e)),
        };
        h.update(&buf[..n]);
        size = size.saturating_add(n as u64);
        progress(size, stamp.size);
    }
    // o arquivo (handle) e o caminho têm de continuar iguais ao início: nem crescimento, nem
    // regravação in-place, nem troca atômica do arquivo por outro
    let after_handle = f.metadata().map_err(io).map(|m| FileStamp::of(&m))?;
    let after_path = file_stamp(path)?;
    if size != stamp.size || after_handle != stamp || after_path != stamp {
        return Err(changed());
    }
    Ok(FileDigest {
        hash: ContentHash(format!("{}{}", ContentHash::PREFIX, hex(&h.finalize()))),
        size,
    })
}

/// Hash de um arquivo regular. Falha se o arquivo mudar **durante** a leitura (import síncrono).
pub fn hash_file(path: &Path) -> Result<FileDigest, AssetError> {
    hash_file_inner(
        path,
        AssetErrorCode::AssetChangedDuringImport,
        &|| false,
        &mut |_, _| {},
    )
}

/// Hash para **jobs em background**: cancelável (checa a cada bloco de 1 MiB), reporta
/// `progress(lidos, total)` e detecta mudança do arquivo durante o processamento
/// (`ASSET_CHANGED_DURING_PROCESSING`). Nunca devolve hash de leitura parcial.
pub fn hash_file_job(
    path: &Path,
    cancel: &dyn Fn() -> bool,
    progress: &mut dyn FnMut(u64, u64),
) -> Result<FileDigest, AssetError> {
    hash_file_inner(
        path,
        AssetErrorCode::AssetChangedDuringProcessing,
        cancel,
        progress,
    )
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn known_vectors() {
        let d = hash_reader(&b""[..]).unwrap();
        assert_eq!(
            d.hash.as_str(),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        let d = hash_reader(&b"abc"[..]).unwrap();
        assert_eq!(
            d.hash.hex(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(d.size, 3);
    }

    #[test]
    fn streaming_across_block_boundaries_matches_one_shot() {
        // 2,5 MiB: atravessa 3 blocos; compara com o hash do buffer inteiro
        let data: Vec<u8> = (0..(2 * BLOCK + BLOCK / 2))
            .map(|i| (i % 251) as u8)
            .collect();
        let streamed = hash_reader(&data[..]).unwrap();
        let mut one = Sha256::new();
        one.update(&data);
        assert_eq!(streamed.hash.hex(), hex(&one.finalize()));
        assert_eq!(streamed.size, data.len() as u64);
    }

    /// Leitor que registra o maior `read` pedido: prova que nunca se pede mais que um bloco.
    struct Spy<R> {
        inner: R,
        max_request: usize,
    }
    impl<R: Read> Read for Spy<R> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.max_request = self.max_request.max(buf.len());
            self.inner.read(buf)
        }
    }

    #[test]
    fn never_asks_for_more_than_one_block() {
        let mut spy = Spy {
            inner: std::io::repeat(7).take(5 * BLOCK as u64),
            max_request: 0,
        };
        let d = hash_reader(&mut spy).unwrap();
        assert_eq!(d.size, 5 * BLOCK as u64);
        assert!(spy.max_request <= BLOCK);
    }

    #[test]
    fn parse_validates_the_format() {
        assert!(ContentHash::parse(d_abc().as_str()).is_some());
        for bad in [
            "",
            "sha256:",
            "sha256:XYZ",
            "md5:abc",
            "sha256:ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789",
            &format!("sha256:{}", "a".repeat(63)),
        ] {
            assert!(ContentHash::parse(bad).is_none(), "{bad}");
        }
    }

    fn d_abc() -> ContentHash {
        hash_reader(&b"abc"[..]).unwrap().hash
    }
}
