//! Localização do arquivo com consciência de plataforma (ADR-048 §3): guarda **o que o SO
//! entrega**, sem trocar separadores, mais um caminho relativo ao projeto sempre com `/`.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetLocation {
    /// Caminho absoluto em texto (UTF-8; *lossy* se o SO usar bytes inválidos).
    pub path: String,
    /// Bytes/unidades exatos quando o caminho não é Unicode válido (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_bytes_hex: Option<String>,
    /// Relativo ao diretório do projeto, componentes separados por `/`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub relative: Option<String>,
}

fn hex(bytes: &[u8]) -> String {
    use core::fmt::Write;
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || s.len() > 16 * 1024 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(unix)]
fn exact_bytes(p: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt;
    p.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
fn exact_bytes(p: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt;
    p.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

#[cfg(unix)]
fn from_exact(b: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(b))
}

#[cfg(windows)]
fn from_exact(b: &[u8]) -> PathBuf {
    use std::os::windows::ffi::OsStringExt;
    let wide: Vec<u16> = b
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    PathBuf::from(std::ffi::OsString::from_wide(&wide))
}

/// Remove `.` e resolve `..` lexicalmente (sem tocar no disco).
fn lexical(p: &Path) -> Vec<Component<'_>> {
    let mut out: Vec<Component<'_>> = Vec::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(out.last(), Some(Component::Normal(_))) {
                    out.pop();
                } else if !matches!(out.last(), Some(Component::RootDir | Component::Prefix(_))) {
                    out.push(c);
                }
            }
            other => out.push(other),
        }
    }
    out
}

/// Caminho de `to` relativo a `from_dir` (ambos absolutos), separado por `/`. `None` se não houver
/// raiz comum (outro drive) ou se algum componente não puder ser representado com segurança.
pub fn relative_path(from_dir: &Path, to: &Path) -> Option<String> {
    let (a, b) = (lexical(from_dir), lexical(to));
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    // sem raiz/prefixo comum (ex.: C: × D:) não existe caminho relativo
    if common == 0 || !matches!(a[0], Component::RootDir | Component::Prefix(_)) {
        return None;
    }
    if matches!(a[0], Component::Prefix(_)) && common < 2 {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..a.len() {
        parts.push("..".into());
    }
    for c in &b[common..] {
        let Component::Normal(n) = c else { return None };
        let s = n.to_str()?;
        if s.is_empty() || s.contains(['/', '\\', ':']) {
            return None;
        }
        parts.push(s.to_owned());
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl AssetLocation {
    pub fn from_path(abs: &Path, project_dir: Option<&Path>) -> Self {
        let (path, path_bytes_hex) = match abs.to_str() {
            Some(s) => (s.to_owned(), None),
            None => (
                abs.to_string_lossy().into_owned(),
                Some(hex(&exact_bytes(abs))),
            ),
        };
        Self {
            path,
            path_bytes_hex,
            relative: project_dir.and_then(|d| relative_path(d, abs)),
        }
    }

    /// O caminho absoluto exato como foi guardado.
    pub fn to_path_buf(&self) -> PathBuf {
        match self.path_bytes_hex.as_deref().and_then(unhex) {
            Some(b) => from_exact(&b),
            None => PathBuf::from(&self.path),
        }
    }
}

fn join_relative(project_dir: &Path, rel: &str) -> Option<PathBuf> {
    if rel.is_empty() || rel.len() > 4096 {
        return None;
    }
    let mut out = project_dir.to_path_buf();
    for c in rel.split('/') {
        match c {
            "" | "." => {}
            ".." => out.push(".."),
            c if c.contains(['\\', ':']) || c.contains('\0') => return None,
            c => out.push(c),
        }
    }
    Some(out)
}

/// Candidatos em ordem de prioridade: relativo ao projeto → absoluto → caminhos conhecidos.
/// Sem duplicatas.
pub fn resolve_candidates(
    loc: &AssetLocation,
    known_paths: &[String],
    project_dir: Option<&Path>,
) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    if let (Some(dir), Some(rel)) = (project_dir, loc.relative.as_deref())
        && let Some(p) = join_relative(dir, rel)
    {
        out.push(p);
    }
    out.push(loc.to_path_buf());
    for k in known_paths {
        let p = PathBuf::from(k);
        if !out.contains(&p) {
            out.push(p);
        }
    }
    let mut seen = Vec::new();
    out.retain(|p| {
        let dup = seen.contains(p);
        if !dup {
            seen.push(p.clone());
        }
        !dup
    });
    out
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn relative_paths_use_forward_slashes_and_parent_steps() {
        let r = |a: &str, b: &str| relative_path(Path::new(a), Path::new(b));
        assert_eq!(
            r("/p/proj", "/p/proj/media/a.mp4").as_deref(),
            Some("media/a.mp4")
        );
        assert_eq!(
            r("/p/proj", "/p/other/a.mp4").as_deref(),
            Some("../other/a.mp4")
        );
        assert_eq!(
            r("/p/proj", "/p/proj/./x/../y.mp4").as_deref(),
            Some("y.mp4")
        );
        assert_eq!(r("/p/proj", "/p/proj"), None);
    }

    #[test]
    fn unicode_and_non_unicode_paths_roundtrip() {
        use std::os::unix::ffi::OsStrExt;
        let u = Path::new("/media/vídeo 日本/ação.mp4");
        let l = AssetLocation::from_path(u, Some(Path::new("/media")));
        assert_eq!(l.to_path_buf(), u);
        assert_eq!(l.relative.as_deref(), Some("vídeo 日本/ação.mp4"));
        assert!(l.path_bytes_hex.is_none());
        let weird = PathBuf::from(std::ffi::OsStr::from_bytes(b"/media/bad-\xff-name.mp4"));
        let l = AssetLocation::from_path(&weird, None);
        assert!(l.path_bytes_hex.is_some());
        assert_eq!(l.to_path_buf(), weird);
    }

    #[test]
    fn hostile_relative_strings_are_ignored() {
        let l = AssetLocation {
            path: "/x/y".into(),
            path_bytes_hex: None,
            relative: Some("a\\b/c:d".into()),
        };
        let c = resolve_candidates(&l, &[], Some(Path::new("/p")));
        assert_eq!(c, vec![PathBuf::from("/x/y")]);
        let l = AssetLocation {
            relative: Some("sub/a.mp4".into()),
            ..l
        };
        let c = resolve_candidates(&l, &["/x/y".into(), "/z".into()], Some(Path::new("/p")));
        assert_eq!(
            c,
            vec![
                PathBuf::from("/p/sub/a.mp4"),
                PathBuf::from("/x/y"),
                PathBuf::from("/z")
            ]
        );
    }

    #[test]
    fn corrupt_hex_falls_back_to_the_text_path() {
        let l = AssetLocation {
            path: "/fallback".into(),
            path_bytes_hex: Some("zz".into()),
            relative: None,
        };
        assert_eq!(l.to_path_buf(), PathBuf::from("/fallback"));
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn drive_letters_decide_whether_a_relative_path_exists() {
        let r = |a: &str, b: &str| relative_path(Path::new(a), Path::new(b));
        assert_eq!(
            r("C:\\p\\proj", "C:\\p\\proj\\media\\a.mp4").as_deref(),
            Some("media/a.mp4")
        );
        assert_eq!(
            r("C:\\p\\proj", "C:\\p\\other\\a.mp4").as_deref(),
            Some("../other/a.mp4")
        );
        // outro drive: não existe caminho relativo
        assert_eq!(r("C:\\p\\proj", "D:\\media\\a.mp4"), None);
    }

    #[test]
    fn unicode_paths_roundtrip_and_resolve_with_native_separators() {
        let u = Path::new("C:\\media\\vídeo 日本\\ação.mp4");
        let l = AssetLocation::from_path(u, Some(Path::new("C:\\media")));
        assert_eq!(l.to_path_buf(), u);
        assert_eq!(l.relative.as_deref(), Some("vídeo 日本/ação.mp4"));
        // o relativo (com `/`) vira caminho nativo ao resolver
        let c = resolve_candidates(&l, &[], Some(Path::new("D:\\other")));
        assert_eq!(
            c[0],
            PathBuf::from("D:\\other")
                .join("vídeo 日本")
                .join("ação.mp4")
        );
    }

    #[test]
    fn hostile_relative_strings_are_ignored() {
        let l = AssetLocation {
            path: "C:\\x\\y".into(),
            path_bytes_hex: None,
            relative: Some("a\\b/c:d".into()),
        };
        let c = resolve_candidates(&l, &[], Some(Path::new("C:\\p")));
        assert_eq!(c, vec![PathBuf::from("C:\\x\\y")]);
    }
}
