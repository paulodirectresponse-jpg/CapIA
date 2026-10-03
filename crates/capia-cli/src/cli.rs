use capia_commands::{Actor, ActorKind, CommandError};
use capia_project::{Project, parse_transaction};
use capia_store::{ProjectInfo, StoreError, StoreOptions, Synchronous, ValidationReport};
use capia_time::TICKS_PER_SECOND;
use serde::Serialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::path::PathBuf;

const USAGE: &str = "\
capia — CapIA engine CLI

USAGE:
  capia create   <projeto.capia>                       cria um projeto vazio
  capia inspect  <projeto.capia> [--json]              resumo (id, schema, sequences, tracks, clips, operações)
  capia validate <projeto.capia> [--json]              integridade SQLite + schema + documento + invariantes
  capia apply    <projeto.capia> <comandos.json|->     aplica comandos (transação, lista ou um comando)
        [--actor user|system|agent|api] [--label TEXTO] [--json]
  capia undo     <projeto.capia> [--json]              desfaz a última entrada
  capia redo     <projeto.capia> [--json]              refaz a última entrada desfeita
  capia history  <projeto.capia> [--json]              entradas de histórico (pilha undo/redo)
  capia dump     <projeto.capia> [--pretty]            documento em JSON canônico e determinístico

ASSETS E MÍDIA:
  capia asset import  <projeto.capia> <arquivo>        importa (hash em streaming + probe); idempotente por conteúdo
  capia asset list    <projeto.capia> [--json]         assets (documento ∪ catálogo) com online/offline/modified
  capia asset inspect <projeto.capia> <asset-id>       registro completo + eventos (import, alias, relink, verify)
  capia asset verify  <projeto.capia> <asset-id>       recalcula o hash e detecta arquivo alterado/ausente
  capia asset relink  <projeto.capia> <asset-id> <arquivo>   só aceita o MESMO conteúdo (senão ASSET_HASH_MISMATCH)
  capia asset thumbnail <projeto.capia> <asset-id> [--at SEGUNDOS] [--size PX]   miniatura no cache
  capia media probe   <arquivo> [--json]               metadados normalizados (ffprobe)
        [--ffprobe CAMINHO] [--ffmpeg CAMINHO] [--timeout-ms N]

OPÇÕES GLOBAIS:
  --sync full|normal|off    durabilidade do SQLite (padrão: full)

CÓDIGOS DE SAÍDA: 0 ok · 1 falha (erro estruturado no stderr) · 2 uso incorreto";

#[derive(Debug, Default)]
pub(crate) struct Args {
    pub(crate) positional: Vec<String>,
    pub(crate) json: bool,
    pub(crate) pretty: bool,
    pub(crate) actor: Option<String>,
    pub(crate) label: Option<String>,
    pub(crate) sync: Option<String>,
    pub(crate) ffprobe: Option<String>,
    pub(crate) ffmpeg: Option<String>,
    pub(crate) timeout_ms: Option<String>,
    pub(crate) at: Option<String>,
    pub(crate) size: Option<String>,
}

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--json" => a.json = true,
            "--pretty" => a.pretty = true,
            "--actor" => a.actor = Some(value("--actor")?),
            "--label" => a.label = Some(value("--label")?),
            "--sync" => a.sync = Some(value("--sync")?),
            "--ffprobe" => a.ffprobe = Some(value("--ffprobe")?),
            "--ffmpeg" => a.ffmpeg = Some(value("--ffmpeg")?),
            "--timeout-ms" => a.timeout_ms = Some(value("--timeout-ms")?),
            "--at" => a.at = Some(value("--at")?),
            "--size" => a.size = Some(value("--size")?),
            flag if flag.starts_with("--") => return Err(format!("unknown option {flag}")),
            _ => a.positional.push(arg.clone()),
        }
    }
    Ok(a)
}

pub(crate) fn options(a: &Args) -> Result<StoreOptions, String> {
    let synchronous = match a.sync.as_deref() {
        None | Some("full") => Synchronous::Full,
        Some("normal") => Synchronous::Normal,
        Some("off") => Synchronous::Off,
        Some(other) => return Err(format!("--sync must be full, normal or off (got {other})")),
    };
    Ok(StoreOptions {
        synchronous,
        ..StoreOptions::default()
    })
}

pub(crate) fn actor(a: &Args) -> Result<Actor, String> {
    Ok(match a.actor.as_deref() {
        None | Some("user") => Actor::new(ActorKind::User, "cli"),
        Some("system") => Actor::system(),
        Some("agent") => Actor::new(ActorKind::Agent, "cli"),
        Some("api") => Actor::new(ActorKind::Api, "cli"),
        Some(other) => {
            return Err(format!(
                "--actor must be user, system, agent or api (got {other})"
            ));
        }
    })
}

pub(crate) fn fmt_duration(ticks: i64) -> String {
    let secs = ticks / TICKS_PER_SECOND;
    let ms = (ticks % TICKS_PER_SECOND) * 1000 / TICKS_PER_SECOND;
    format!("{secs}.{ms:03} s")
}

pub(crate) struct Io<'a> {
    pub(crate) out: &'a mut dyn Write,
    pub(crate) err: &'a mut dyn Write,
}

impl Io<'_> {
    pub(crate) fn print(&mut self, text: &str) {
        let _ = writeln!(self.out, "{text}");
    }

    pub(crate) fn print_json<T: Serialize>(&mut self, value: &T, pretty: bool) {
        // `Value` tem chaves ordenadas ⇒ saída determinística
        let v = serde_json::to_value(value).unwrap_or(Value::Null);
        let text = if pretty {
            serde_json::to_string_pretty(&v)
        } else {
            serde_json::to_string(&v)
        };
        self.print(&text.unwrap_or_default());
    }

    pub(crate) fn fail<T: Serialize>(&mut self, e: &T, summary: &str, json: bool) -> i32 {
        if json {
            let v = json!({ "error": serde_json::to_value(e).unwrap_or(Value::Null) });
            let _ = writeln!(self.err, "{v}");
        } else {
            let _ = writeln!(self.err, "error: {summary}");
            if let Ok(v) = serde_json::to_string(e) {
                let _ = writeln!(self.err, "{v}");
            }
        }
        1
    }

    fn store_err(&mut self, e: &StoreError, json: bool) -> i32 {
        self.fail(e, &format!("{e}"), json)
    }

    fn cmd_err(&mut self, e: &CommandError, json: bool) -> i32 {
        self.fail(e, &format!("{e}"), json)
    }
}

fn print_info(io: &mut Io<'_>, info: &ProjectInfo) {
    io.print(&format!("project:   {}", info.project_id));
    io.print(&format!("file:      {}", info.path));
    io.print(&format!(
        "schema:    {} (supported {}){}",
        info.schema_version,
        info.supported_schema_version,
        if info.needs_migration {
            " — needs migration"
        } else {
            ""
        }
    ));
    io.print(&format!(
        "revision:  {}  digest {}",
        info.revision, info.digest
    ));
    io.print(&format!(
        "history:   {} entries · {} operations · {} events (undo {}, redo {})",
        info.stats.history_entries,
        info.stats.operations,
        info.stats.events,
        info.stats.undo_depth,
        info.stats.redo_depth
    ));
    io.print(&format!(
        "document:  {} sequences · {} tracks · {} clips · {} assets ({} with media files)",
        info.sequences.len(),
        info.total_tracks,
        info.total_clips,
        info.assets,
        info.media_assets
    ));
    for s in &info.sequences {
        io.print(&format!(
            "  {} \"{}\"  {} fps · {} tracks · {} clips · duration {} ({} ticks)",
            s.id,
            s.name,
            s.frame_rate,
            s.tracks,
            s.clips,
            fmt_duration(s.duration_ticks),
            s.duration_ticks
        ));
    }
}

/// Executa um comando da CLI. Devolve o código de saída.
pub(crate) fn run(raw: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let mut io = Io { out, err };
    let Some((cmd, rest)) = raw.split_first() else {
        let _ = writeln!(io.err, "{USAGE}");
        return 2;
    };
    if matches!(cmd.as_str(), "help" | "--help" | "-h") {
        io.print(USAGE);
        return 0;
    }
    let usage = |io: &mut Io<'_>, msg: &str| {
        let _ = writeln!(io.err, "error: {msg}\n\n{USAGE}");
        2
    };
    let args = match parse_args(rest) {
        Ok(a) => a,
        Err(m) => return usage(&mut io, &m),
    };
    let opts = match options(&args) {
        Ok(o) => o,
        Err(m) => return usage(&mut io, &m),
    };
    if matches!(cmd.as_str(), "asset" | "media") {
        return crate::assets_cmd::run(cmd, &args, &opts, &mut io);
    }
    let Some(path) = args.positional.first().map(PathBuf::from) else {
        return usage(&mut io, "missing project path");
    };
    let json = args.json;
    let extra = args.positional.len().saturating_sub(1);
    let expect_extra = usize::from(cmd == "apply");
    if extra != expect_extra {
        return usage(&mut io, &format!("unexpected arguments for `{cmd}`"));
    }

    match cmd.as_str() {
        "create" => match Project::create(&path, &opts) {
            Ok(project) => {
                drop(project);
                match Project::inspect(&path) {
                    Ok(info) if json => {
                        io.print_json(&info, false);
                        0
                    }
                    Ok(info) => {
                        io.print(&format!("created {}", path.display()));
                        print_info(&mut io, &info);
                        0
                    }
                    Err(e) => io.store_err(&e, json),
                }
            }
            Err(e) => io.store_err(&e, json),
        },
        "inspect" => match Project::inspect(&path) {
            Ok(info) if json => {
                io.print_json(&info, args.pretty);
                0
            }
            Ok(info) => {
                print_info(&mut io, &info);
                0
            }
            Err(e) => io.store_err(&e, json),
        },
        "validate" => {
            let report: ValidationReport = Project::validate(&path);
            if json {
                io.print_json(&report, args.pretty);
            } else if report.ok {
                io.print(&format!("OK: {} is a valid CapIA project", path.display()));
                if let Some(info) = &report.info {
                    print_info(&mut io, info);
                }
            } else {
                io.print(&format!("INVALID: {}", path.display()));
                for issue in &report.issues {
                    io.print(&format!("  - {issue}"));
                }
            }
            i32::from(!report.ok)
        }
        "apply" => {
            let source = &args.positional[1];
            let mut text = String::new();
            let read = if source == "-" {
                std::io::stdin().read_to_string(&mut text).map(|_| ())
            } else {
                std::fs::read_to_string(source).map(|t| text = t)
            };
            if let Err(e) = read {
                return usage(&mut io, &format!("cannot read {source}: {e}"));
            }
            let label = args
                .label
                .clone()
                .unwrap_or_else(|| "capia apply".to_owned());
            let tx = match parse_transaction(&text, &label) {
                Ok(t) => t,
                Err(e) => {
                    return io.fail(
                        &json!({ "code": "INVALID_ARGUMENT", "message": e.0 }),
                        &e.0,
                        json,
                    );
                }
            };
            let who = match actor(&args) {
                Ok(a) => a,
                Err(m) => return usage(&mut io, &m),
            };
            let mut project = match Project::open(&path, &opts) {
                Ok(p) => p,
                Err(e) => return io.store_err(&e, json),
            };
            // atores Agent/Api só escrevem por preview → apply_plan (ADR-030): a CLI faz os dois
            let result = if who.requires_preview() {
                match project.preview(&who, tx) {
                    Ok(p) if p.already_applied => {
                        if json {
                            io.print_json(&json!({ "already_applied": true }), false);
                        } else {
                            io.print("already applied (same operation_ids): nothing to do");
                        }
                        return 0;
                    }
                    Ok(p) => project.apply_plan(&who, p.plan_token.as_deref().unwrap_or_default()),
                    Err(e) => Err(e),
                }
            } else {
                project.execute(&who, tx)
            };
            match result {
                Ok(r) => {
                    if json {
                        io.print_json(&r, args.pretty);
                    } else {
                        io.print(&format!(
                            "{} entry {} · revision {} → {}{}",
                            if r.replayed {
                                "replayed (already applied)"
                            } else {
                                "applied"
                            },
                            r.entry_id,
                            r.revision_before,
                            r.revision,
                            if r.refs.is_empty() {
                                String::new()
                            } else {
                                format!(" · refs {:?}", r.refs)
                            }
                        ));
                        for w in r.results.iter().flat_map(|o| &o.warnings) {
                            io.print(&format!("warning: {w}"));
                        }
                    }
                    0
                }
                Err(e) => io.cmd_err(&e, json),
            }
        }
        "undo" | "redo" => {
            let mut project = match Project::open(&path, &opts) {
                Ok(p) => p,
                Err(e) => return io.store_err(&e, json),
            };
            let who = Actor::new(ActorKind::User, "cli");
            let result = if cmd == "undo" {
                project.undo(&who)
            } else {
                project.redo(&who)
            };
            match result {
                Ok(r) => {
                    if json {
                        io.print_json(&r, args.pretty);
                    } else {
                        io.print(&format!(
                            "{cmd} entry {} · revision {} → {}",
                            r.entry_id, r.revision_before, r.revision
                        ));
                    }
                    0
                }
                Err(e) => io.cmd_err(&e, json),
            }
        }
        "history" => {
            let project = match Project::open(&path, &opts) {
                Ok(p) => p,
                Err(e) => return io.store_err(&e, json),
            };
            let engine = project.engine();
            let applied = engine.applied_history().len();
            let rows: Vec<Value> = engine
                .history()
                .iter()
                .enumerate()
                .map(|(i, h)| {
                    json!({
                        "entry": h.id,
                        "label": h.label,
                        "actor": h.actor,
                        "revision_after": h.revision_after,
                        "commands": h.commands.iter().map(|c| &c.command_type).collect::<Vec<_>>(),
                        "state": if i < applied { "applied" } else { "undone" },
                    })
                })
                .collect();
            if json {
                io.print_json(&rows, args.pretty);
            } else {
                for r in &rows {
                    io.print(&format!(
                        "#{:<4} {:<8} rev {:<5} {}  [{}]",
                        r["entry"],
                        r["state"].as_str().unwrap_or(""),
                        r["revision_after"],
                        r["label"].as_str().unwrap_or(""),
                        r["commands"].as_array().map_or(String::new(), |c| c
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<Vec<_>>()
                            .join(", "))
                    ));
                }
                if rows.is_empty() {
                    io.print("(no history)");
                }
            }
            0
        }
        "dump" => match Project::open(&path, &opts) {
            Ok(project) => {
                io.print_json(project.document(), args.pretty);
                0
            }
            Err(e) => io.store_err(&e, json),
        },
        other => usage(&mut io, &format!("unknown command `{other}`")),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parses_flags_and_positionals() {
        let args = parse_args(&a(&["p.capia", "--json", "--actor", "agent", "cmd.json"])).unwrap();
        assert_eq!(args.positional, ["p.capia", "cmd.json"]);
        assert!(args.json && !args.pretty);
        assert_eq!(args.actor.as_deref(), Some("agent"));
    }

    #[test]
    fn rejects_unknown_flags_and_missing_values() {
        assert!(parse_args(&a(&["--nope"])).is_err());
        assert!(parse_args(&a(&["p", "--actor"])).is_err());
    }

    #[test]
    fn validates_actor_and_sync_choices() {
        let bad = parse_args(&a(&["p", "--actor", "root"])).unwrap();
        assert!(actor(&bad).is_err());
        let bad = parse_args(&a(&["p", "--sync", "maybe"])).unwrap();
        assert!(options(&bad).is_err());
        let ok = parse_args(&a(&["p", "--sync", "off"])).unwrap();
        assert_eq!(options(&ok).unwrap().synchronous, Synchronous::Off);
    }

    #[test]
    fn durations_are_formatted_with_integer_math() {
        assert_eq!(fmt_duration(0), "0.000 s");
        assert_eq!(
            fmt_duration(TICKS_PER_SECOND * 3 + TICKS_PER_SECOND / 2),
            "3.500 s"
        );
        assert_eq!(fmt_duration(23_520_000), "0.033 s");
    }
}
