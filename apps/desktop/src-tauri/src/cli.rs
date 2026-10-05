//! Flags de linha de comando seguras do binário desktop (sem abrir janela):
//!
//! * `--version` → versão/build (`--json` para JSON);
//! * `--self-test [--require-media]` → verificação headless (ver `selftest`), relatório JSON, saída 0/1.
//! * `--out <arquivo>` → grava o resultado também (ou só) no arquivo: o executável de release usa o subsistema
//!   `windows` e **não tem console**, então o instalador lê o relatório do arquivo e o código de saída.
//!
//! Nenhuma outra flag é tratada aqui (o WebView2/Tauri podem receber as suas); sem flags conhecidas o app abre normal.

use crate::selftest::{self, SelfTestOptions};
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Run,
    Version { json: bool },
    SelfTest { require_media: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cli {
    pub action: Action,
    pub out: Option<PathBuf>,
}

pub fn parse(args: &[String]) -> Cli {
    let mut action = Action::Run;
    let mut out = None;
    let mut json = false;
    let mut require_media = false;
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        match a.as_str() {
            "--version" | "-V" => action = Action::Version { json: false },
            "--self-test" => {
                action = Action::SelfTest {
                    require_media: false,
                }
            }
            "--require-media" => require_media = true,
            "--json" => json = true,
            "--out" => out = it.next().map(PathBuf::from),
            _ => {}
        }
    }
    match &mut action {
        Action::Version { json: j } => *j = json,
        Action::SelfTest { require_media: r } => *r = require_media,
        Action::Run => {}
    }
    Cli { action, out }
}

fn emit(text: &str, out: Option<&PathBuf>) {
    if let Some(p) = out {
        let _ = std::fs::write(p, text);
    }
    // no Windows com `windows_subsystem = "windows"` isto é inofensivo (sem console)
    println!("{text}");
}

/// Trata as flags; `Some(código)` = o processo deve sair com ele, `None` = abrir o app.
pub fn handle(args: &[String]) -> Option<i32> {
    let cli = parse(args);
    match cli.action {
        Action::Run => None,
        Action::Version { json } => {
            let b = capia_support::BuildInfo::current();
            let text = if json {
                serde_json::to_string_pretty(&b).unwrap_or_default()
            } else {
                format!("CapIA {}", b.display())
            };
            emit(&text, cli.out.as_ref());
            Some(0)
        }
        Action::SelfTest { require_media } => {
            let (ok, report) = selftest::run(&SelfTestOptions { require_media });
            emit(
                &serde_json::to_string_pretty(&report).unwrap_or_default(),
                cli.out.as_ref(),
            );
            Some(i32::from(!ok))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn parses_known_flags_and_ignores_the_rest() {
        assert_eq!(parse(&a(&["capia"])).action, Action::Run);
        assert_eq!(
            parse(&a(&["capia", "--remote-debugging-port=9222"])).action,
            Action::Run
        );
        assert_eq!(
            parse(&a(&["capia", "--version", "--json"])).action,
            Action::Version { json: true }
        );
        let c = parse(&a(&[
            "capia",
            "--self-test",
            "--require-media",
            "--out",
            "r.json",
        ]));
        assert_eq!(
            c.action,
            Action::SelfTest {
                require_media: true
            }
        );
        assert_eq!(c.out, Some(PathBuf::from("r.json")));
        // a ordem das flags não importa
        assert_eq!(
            parse(&a(&["capia", "--require-media", "--self-test"])).action,
            Action::SelfTest {
                require_media: true
            }
        );
    }

    #[test]
    fn version_writes_the_out_file_and_exits_zero() {
        let p = std::env::temp_dir().join(format!("capia-ver-{}.txt", std::process::id()));
        let code = handle(&a(&[
            "capia",
            "--version",
            "--out",
            &p.display().to_string(),
        ]));
        assert_eq!(code, Some(0));
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let _ = std::fs::remove_file(&p);
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
    }
}
