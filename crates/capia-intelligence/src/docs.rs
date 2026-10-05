//! Extração **segura** de texto de documentos de briefing (TXT/MD/DOCX/PDF). O arquivo é entrada
//! hostil: tamanho limitado, ZIP com teto de entradas/bytes descomprimidos (anti zip-bomb), PDF num
//! thread com `catch_unwind` e prazo (o parser é de terceiros), nada é executado, nenhuma URL é
//! seguida. O resultado são **unidades endereçáveis** (`unit_id`) — a base da rastreabilidade do
//! `DemandSpec`: toda afirmação aponta para uma unidade e uma citação literal.

use crate::error::{IntelError, IntelResult};
use quick_xml::Reader;
use quick_xml::events::Event;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::Path;
use std::time::Duration;

pub const MAX_FILE_BYTES: u64 = 50 * 1024 * 1024;
pub const MAX_TEXT_CHARS: usize = 2_000_000;
pub const MAX_UNITS: usize = 50_000;
pub const MAX_UNIT_CHARS: usize = 1_500;
const MAX_ZIP_ENTRIES: usize = 4_000;
const MAX_XML_BYTES: u64 = 64 * 1024 * 1024;
const PDF_DEADLINE: Duration = Duration::from_secs(60);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    Txt,
    Markdown,
    Docx,
    Pdf,
    Transcript,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unit {
    /// `u1`, `u2`, … (estável para o mesmo conteúdo).
    pub id: String,
    /// Página (PDF) quando conhecida.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// Instante de origem (µs) para unidades vindas de transcrição.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_us: Option<i64>,
    #[serde(default)]
    pub heading: bool,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractedDoc {
    /// `sha256:<hex>` do **conteúdo** do arquivo (identidade, nunca o caminho).
    pub id: String,
    pub name: String,
    pub kind: DocKind,
    pub units: Vec<Unit>,
    pub truncated: bool,
}

impl ExtractedDoc {
    pub fn unit(&self, id: &str) -> Option<&Unit> {
        self.units.iter().find(|u| u.id == id)
    }

    pub fn char_count(&self) -> usize {
        self.units.iter().map(|u| u.text.chars().count()).sum()
    }
}

fn err(code: &str, msg: impl Into<String>) -> IntelError {
    IntelError::new(code, msg)
}

fn sha(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    format!("sha256:{}", capia_ai::types::hex(&d))
}

fn sanitize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect::<String>()
        .replace('\u{00a0}', " ")
}

/// Quebra texto corrido em unidades (parágrafos; pedaços longos são cortados em fronteira de
/// frase/palavra), numeradas a partir de `first_id`.
fn push_units(
    out: &mut Vec<Unit>,
    text: &str,
    page: Option<u32>,
    heading: bool,
    truncated: &mut bool,
) {
    for para in text.split("\n\n") {
        let p = para.split_whitespace().collect::<Vec<_>>().join(" ");
        if p.is_empty() {
            continue;
        }
        let mut rest: &str = &p;
        while !rest.is_empty() {
            if out.len() >= MAX_UNITS {
                *truncated = true;
                return;
            }
            let take = if rest.chars().count() <= MAX_UNIT_CHARS {
                rest.len()
            } else {
                let cut: usize = rest
                    .char_indices()
                    .nth(MAX_UNIT_CHARS)
                    .map_or(rest.len(), |(i, _)| i);
                let head = &rest[..cut];
                head.rfind(". ")
                    .map(|i| i + 1)
                    .or_else(|| head.rfind(' '))
                    .filter(|i| *i > cut / 2)
                    .unwrap_or(cut)
            };
            let (chunk, tail) = rest.split_at(take);
            out.push(Unit {
                id: format!("u{}", out.len() + 1),
                page,
                t_us: None,
                heading,
                text: chunk.trim().to_owned(),
            });
            rest = tail.trim_start();
        }
    }
}

pub fn extract_text(name: &str, kind: DocKind, text: &str, id: String) -> ExtractedDoc {
    let mut truncated = false;
    let text: String = sanitize(text).chars().take(MAX_TEXT_CHARS).collect();
    let mut units = Vec::new();
    push_units(&mut units, &text, None, false, &mut truncated);
    ExtractedDoc {
        id,
        name: name.to_owned(),
        kind,
        units,
        truncated,
    }
}

fn extract_docx(name: &str, bytes: &[u8], id: String) -> IntelResult<ExtractedDoc> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|_| err("DOC_INVALID", "the DOCX is not a valid ZIP container"))?;
    if zip.len() > MAX_ZIP_ENTRIES {
        return Err(err("DOC_LIMIT", "the DOCX has too many entries"));
    }
    let mut xml = Vec::new();
    {
        let f = zip
            .by_name("word/document.xml")
            .map_err(|_| err("DOC_INVALID", "the DOCX has no word/document.xml"))?;
        if f.size() > MAX_XML_BYTES {
            return Err(err("DOC_LIMIT", "word/document.xml is too large"));
        }
        // o tamanho declarado no cabeçalho pode mentir: o limite real é no `take`
        f.take(MAX_XML_BYTES + 1)
            .read_to_end(&mut xml)
            .map_err(|_| err("DOC_INVALID", "the DOCX could not be decompressed"))?;
        if xml.len() as u64 > MAX_XML_BYTES {
            return Err(err("DOC_LIMIT", "word/document.xml exceeds the size limit"));
        }
    }
    let mut reader = Reader::from_reader(xml.as_slice());
    let mut units: Vec<Unit> = Vec::new();
    let mut truncated = false;
    let (mut cur, mut in_t, mut heading) = (String::new(), false, false);
    let mut total = 0usize;
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => match e.local_name().as_ref() {
                b"p" => {
                    cur.clear();
                    heading = false;
                }
                b"t" => in_t = true,
                b"pStyle" => heading |= style_is_heading(&e),
                b"br" => cur.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(e)) => match e.local_name().as_ref() {
                b"tab" => cur.push('\t'),
                b"br" => cur.push('\n'),
                b"pStyle" => heading |= style_is_heading(&e),
                _ => {}
            },
            Ok(Event::Text(t)) if in_t => {
                if let Ok(s) = t.unescape() {
                    total += s.chars().count();
                    if total > MAX_TEXT_CHARS {
                        truncated = true;
                        break;
                    }
                    cur.push_str(&s);
                }
            }
            Ok(Event::End(e)) => match e.local_name().as_ref() {
                b"t" => in_t = false,
                b"p" => {
                    let t = sanitize(&cur);
                    push_units(&mut units, &t, None, heading, &mut truncated);
                    cur.clear();
                    if truncated {
                        break;
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(_) => return Err(err("DOC_INVALID", "the DOCX XML is malformed")),
            _ => {}
        }
        buf.clear();
    }
    // `push_units` numera por posição: renumera para ids contíguos e estáveis
    for (i, u) in units.iter_mut().enumerate() {
        u.id = format!("u{}", i + 1);
    }
    Ok(ExtractedDoc {
        id,
        name: name.to_owned(),
        kind: DocKind::Docx,
        units,
        truncated,
    })
}

fn style_is_heading(e: &quick_xml::events::BytesStart<'_>) -> bool {
    e.attributes().flatten().any(|a| {
        a.key.local_name().as_ref() == b"val"
            && String::from_utf8_lossy(&a.value)
                .to_ascii_lowercase()
                .starts_with("heading")
    })
}

fn extract_pdf(name: &str, bytes: Vec<u8>, id: String) -> IntelResult<ExtractedDoc> {
    let (tx, rx) = std::sync::mpsc::channel();
    // O parser é de terceiros: um thread isolado, com `catch_unwind` e prazo. Se estourar o prazo
    // o thread é abandonado (não há como matá-lo) — o app segue funcionando.
    std::thread::Builder::new()
        .name("capia-pdf-extract".into())
        .spawn(move || {
            let r =
                std::panic::catch_unwind(|| pdf_extract::extract_text_from_mem_by_pages(&bytes));
            let _ = tx.send(r);
        })
        .map_err(|_| err("INTERNAL", "could not start the PDF extraction thread"))?;
    let text = match rx.recv_timeout(PDF_DEADLINE) {
        Ok(Ok(Ok(t))) => t,
        Ok(Ok(Err(_))) | Ok(Err(_)) => {
            return Err(err("DOC_INVALID", "the PDF could not be parsed"));
        }
        Err(_) => return Err(err("DOC_TIMEOUT", "the PDF extraction took too long")),
    };
    let mut units = Vec::new();
    let mut truncated = false;
    let mut total = 0usize;
    for (i, page) in text.iter().enumerate() {
        let page_text = sanitize(page);
        total += page_text.chars().count();
        if total > MAX_TEXT_CHARS {
            truncated = true;
            break;
        }
        // PDFs trazem quebra por linha: junta linhas de um mesmo parágrafo
        let joined = page_text
            .split("\n\n")
            .map(|p| p.replace('\n', " "))
            .collect::<Vec<_>>()
            .join("\n\n");
        push_units(
            &mut units,
            &joined,
            Some(u32::try_from(i + 1).unwrap_or(u32::MAX)),
            false,
            &mut truncated,
        );
        if truncated {
            break;
        }
    }
    for (i, u) in units.iter_mut().enumerate() {
        u.id = format!("u{}", i + 1);
    }
    Ok(ExtractedDoc {
        id,
        name: name.to_owned(),
        kind: DocKind::Pdf,
        units,
        truncated,
    })
}

/// Lê um arquivo de briefing (formato pela extensão **e** pela assinatura).
pub fn extract_file(path: &Path) -> IntelResult<ExtractedDoc> {
    let meta =
        std::fs::metadata(path).map_err(|_| err("DOC_NOT_FOUND", "the file was not found"))?;
    if !meta.is_file() {
        return Err(err("DOC_INVALID", "not a regular file"));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(err("DOC_LIMIT", "the file is larger than 50 MB"));
    }
    let bytes =
        std::fs::read(path).map_err(|_| err("DOC_NOT_FOUND", "the file could not be read"))?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let id = sha(&bytes);
    match ext.as_str() {
        "docx" => {
            if !bytes.starts_with(b"PK\x03\x04") {
                return Err(err("DOC_INVALID", "the file is not a DOCX (bad signature)"));
            }
            extract_docx(&name, &bytes, id)
        }
        "pdf" => {
            if !bytes.starts_with(b"%PDF-") {
                return Err(err("DOC_INVALID", "the file is not a PDF (bad signature)"));
            }
            extract_pdf(&name, bytes, id)
        }
        "txt" | "md" | "markdown" | "text" => {
            let kind = if ext.starts_with('m') {
                DocKind::Markdown
            } else {
                DocKind::Txt
            };
            Ok(extract_text(
                &name,
                kind,
                &String::from_utf8_lossy(&bytes),
                id,
            ))
        }
        _ => Err(err(
            "DOC_UNSUPPORTED",
            format!("unsupported document type `.{ext}` (use .docx, .pdf, .txt or .md)"),
        )),
    }
}

/// Transcrição → unidades (uma por segmento), com o instante de origem.
pub fn extract_transcript(
    name: &str,
    asset_id: &str,
    t: &capia_ai::stt::Transcript,
) -> ExtractedDoc {
    let mut units = Vec::new();
    for s in &t.segments {
        let text = sanitize(&s.text)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() || units.len() >= MAX_UNITS {
            continue;
        }
        units.push(Unit {
            id: format!("u{}", units.len() + 1),
            page: None,
            t_us: Some(s.start_us),
            heading: false,
            text,
        });
    }
    ExtractedDoc {
        id: format!("transcript:{asset_id}"),
        name: name.to_owned(),
        kind: DocKind::Transcript,
        units,
        truncated: false,
    }
}

/// Geradores de documentos mínimos (DOCX/PDF) para testes — sem arquivos binários versionados.
#[cfg(any(test, feature = "testkit"))]
#[allow(clippy::unwrap_used)]
pub mod testing {
    use std::io::Write;

    pub fn make_docx(body_xml: &str) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default();
            z.start_file("word/document.xml", o).unwrap();
            z.write_all(
                format!(
                    r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>{body_xml}</w:body></w:document>"#
                )
                .as_bytes(),
            )
            .unwrap();
            z.finish().unwrap();
        }
        buf.into_inner()
    }

    /// PDF mínimo e válido (Helvetica) com uma página por texto; offsets do xref calculados.
    pub fn make_pdf(pages: &[&str]) -> Vec<u8> {
        let mut out: Vec<u8> = b"%PDF-1.4\n".to_vec();
        let mut offs: Vec<usize> = Vec::new();
        let n = pages.len();
        // objetos: 1 catalog, 2 pages, 3 font, depois (page, content) por página
        let mut add = |out: &mut Vec<u8>, body: String| {
            offs.push(out.len());
            let id = offs.len();
            out.extend_from_slice(format!("{id} 0 obj\n{body}\nendobj\n").as_bytes());
        };
        add(&mut out, "<< /Type /Catalog /Pages 2 0 R >>".into());
        let kids: String = (0..n).map(|i| format!("{} 0 R ", 4 + 2 * i)).collect();
        add(
            &mut out,
            format!("<< /Type /Pages /Kids [{kids}] /Count {n} >>"),
        );
        add(
            &mut out,
            "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".into(),
        );
        for (i, t) in pages.iter().enumerate() {
            let content = format!("BT /F1 12 Tf 72 720 Td ({t}) Tj ET");
            add(
                &mut out,
                format!(
                    "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {} 0 R /Resources << /Font << /F1 3 0 R >> >> >>",
                    5 + 2 * i
                ),
            );
            add(
                &mut out,
                format!(
                    "<< /Length {} >>\nstream\n{content}\nendstream",
                    content.len()
                ),
            );
        }
        let xref = out.len();
        out.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", offs.len() + 1).as_bytes(),
        );
        for o in &offs {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                offs.len() + 1
            )
            .as_bytes(),
        );
        out
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use super::testing::{make_docx as docx, make_pdf};
    use super::*;
    use std::io::Write;

    #[test]
    fn docx_paragraphs_headings_tabs_and_entities() {
        let b = docx(
            r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Briefing</w:t></w:r></w:p>
               <w:p><w:r><w:t>Produto: </w:t></w:r><w:r><w:t xml:space="preserve">Café &amp; Cia</w:t></w:r></w:p>
               <w:p><w:r><w:t>Público</w:t><w:tab/><w:t>mulheres 25-40</w:t></w:r></w:p>
               <w:p></w:p>"#,
        );
        let d = extract_docx("b.docx", &b, "sha256:x".into()).unwrap();
        assert_eq!(d.units.len(), 3, "{:?}", d.units);
        assert!(d.units[0].heading && d.units[0].text == "Briefing");
        assert_eq!(d.units[1].text, "Produto: Café & Cia");
        assert_eq!(d.units[2].text, "Público mulheres 25-40");
        assert_eq!(d.units[2].id, "u3");
    }

    #[test]
    fn docx_rejects_non_zip_missing_part_and_bombs() {
        assert_eq!(
            extract_docx("x", b"not a zip", String::new())
                .unwrap_err()
                .code,
            "DOC_INVALID"
        );
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut buf);
            z.start_file("other.xml", zip::write::SimpleFileOptions::default())
                .unwrap();
            z.write_all(b"<a/>").unwrap();
            z.finish().unwrap();
        }
        assert_eq!(
            extract_docx("x", buf.get_ref(), String::new())
                .unwrap_err()
                .code,
            "DOC_INVALID"
        );
        // bomba: 70 MB de zeros comprimem para ~70 KB; o limite de 64 MB descomprimidos barra
        let mut bomb = std::io::Cursor::new(Vec::new());
        {
            let mut z = zip::ZipWriter::new(&mut bomb);
            z.start_file(
                "word/document.xml",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Deflated),
            )
            .unwrap();
            let chunk = vec![b' '; 1 << 20];
            for _ in 0..70 {
                z.write_all(&chunk).unwrap();
            }
            z.finish().unwrap();
        }
        assert_eq!(
            extract_docx("x", bomb.get_ref(), String::new())
                .unwrap_err()
                .code,
            "DOC_LIMIT"
        );
    }

    #[test]
    fn text_is_split_in_bounded_units_and_control_chars_are_stripped() {
        let long = "palavra ".repeat(600); // 4800 chars
        let d = extract_text(
            "t.txt",
            DocKind::Txt,
            &format!("um\u{0}\u{7}dois\n\n{long}\n\nfim"),
            "sha256:y".into(),
        );
        assert!(
            d.units
                .iter()
                .all(|u| u.text.chars().count() <= MAX_UNIT_CHARS)
        );
        assert!(d.units.len() >= 5, "{}", d.units.len());
        assert_eq!(d.units[0].text, "umdois");
        assert_eq!(d.units.last().unwrap().text, "fim");
        let ids: Vec<_> = d.units.iter().map(|u| u.id.clone()).collect();
        assert_eq!(ids[0], "u1");
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::BTreeSet<_>>().len()
        );
    }

    #[test]
    fn file_type_is_checked_by_extension_and_signature() {
        let dir = std::env::temp_dir().join(format!("capia-docs-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("fake.docx");
        std::fs::write(&fake, b"hello").unwrap();
        assert_eq!(extract_file(&fake).unwrap_err().code, "DOC_INVALID");
        let fakepdf = dir.join("fake.pdf");
        std::fs::write(&fakepdf, b"hello").unwrap();
        assert_eq!(extract_file(&fakepdf).unwrap_err().code, "DOC_INVALID");
        let exe = dir.join("a.exe");
        std::fs::write(&exe, b"MZ").unwrap();
        assert_eq!(extract_file(&exe).unwrap_err().code, "DOC_UNSUPPORTED");
        assert_eq!(
            extract_file(&dir.join("nope.txt")).unwrap_err().code,
            "DOC_NOT_FOUND"
        );
        let ok = dir.join("ok.md");
        std::fs::write(&ok, "# Título\n\nTexto.").unwrap();
        let d = extract_file(&ok).unwrap();
        assert_eq!(d.kind, DocKind::Markdown);
        assert!(d.id.starts_with("sha256:"));
        // mesma conteúdo ⇒ mesmo id, independente do nome
        let copy = dir.join("copy.md");
        std::fs::copy(&ok, &copy).unwrap();
        assert_eq!(extract_file(&copy).unwrap().id, d.id);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pdf_text_is_extracted_per_page_and_garbage_is_rejected() {
        let d = extract_pdf(
            "b.pdf",
            make_pdf(&["Oferta valida ate sexta", "Garantia de 30 dias"]),
            "sha256:p".into(),
        )
        .unwrap();
        let all: String = d
            .units
            .iter()
            .map(|u| u.text.as_str())
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(all.contains("Oferta valida ate sexta"), "{all}");
        assert!(all.contains("Garantia de 30 dias"), "{all}");
        assert_eq!(d.units.first().unwrap().page, Some(1));
        assert!(d.units.iter().any(|u| u.page == Some(2)), "{:?}", d.units);
        // lixo com a assinatura certa não derruba o processo
        let e = extract_pdf(
            "g.pdf",
            b"%PDF-1.4\n\x00\x01garbage".to_vec(),
            String::new(),
        );
        assert!(e.is_err() || e.unwrap().units.is_empty());
    }

    #[test]
    fn transcript_units_carry_their_origin_time() {
        let t = capia_ai::stt::Transcript {
            schema_version: 1,
            language: None,
            duration_us: None,
            segments: vec![capia_ai::stt::Segment {
                start_us: 4_200_000,
                end_us: 6_000_000,
                text: "  compre   agora ".into(),
                confidence: None,
                speaker: None,
                words: vec![],
            }],
        };
        let d = extract_transcript("v.mp4", "asset1", &t);
        assert_eq!(d.units[0].t_us, Some(4_200_000));
        assert_eq!(d.units[0].text, "compre agora");
        assert_eq!(d.id, "transcript:asset1");
    }
}
