//! Medição honesta: amostras brutas, percentis por posto mais próximo (nearest-rank), informação da
//! máquina e relatório JSON. Nada aqui "melhora" números: o relatório guarda as amostras.

use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Estatística de uma série de tempos em milissegundos.
#[derive(Clone, Debug, PartialEq)]
pub struct Stat {
    pub n: usize,
    pub min: f64,
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
    pub mean: f64,
}

/// Percentil por posto mais próximo (`p` em `0..=100`) de `sorted` (ordenado, não vazio).
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    assert!(!sorted.is_empty(), "percentile of an empty series");
    let rank = ((p / 100.0) * sorted.len() as f64).ceil() as usize;
    sorted[rank.clamp(1, sorted.len()) - 1]
}

impl Stat {
    pub fn of(samples_ms: &[f64]) -> Self {
        assert!(!samples_ms.is_empty(), "no samples");
        let mut s = samples_ms.to_vec();
        s.sort_by(f64::total_cmp);
        Self {
            n: s.len(),
            min: s[0],
            p50: percentile(&s, 50.0),
            p95: percentile(&s, 95.0),
            max: s[s.len() - 1],
            mean: s.iter().sum::<f64>() / s.len() as f64,
        }
    }
}

pub fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// Mede `f` `reps` vezes e devolve os tempos em ms. O resultado de `f` passa por `black_box`.
pub fn time_reps<T>(reps: usize, mut f: impl FnMut(usize) -> T) -> Vec<f64> {
    (0..reps)
        .map(|i| {
            let t = Instant::now();
            let out = f(i);
            let el = t.elapsed();
            std::hint::black_box(out);
            ms(el)
        })
        .collect()
}

/// Como [`time_reps`], mas a própria `f` devolve os ms a registrar (para medir só um trecho do
/// corpo, p.ex. excluindo o preparo/limpeza da repetição).
pub fn ms_reps(reps: usize, mut f: impl FnMut(usize) -> f64) -> Vec<f64> {
    (0..reps).map(&mut f).collect()
}

/// Dados da máquina que produziu os números (sem isto um número não é comparável).
pub fn machine_info() -> Value {
    let cpuinfo = std::fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu = cpuinfo
        .lines()
        .find(|l| l.starts_with("model name"))
        .and_then(|l| l.split_once(':'))
        .map_or_else(|| "unknown".to_owned(), |(_, v)| v.trim().to_owned());
    let meminfo = std::fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mem_kib: Option<u64> = meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|v| v.parse().ok());
    let kernel = std::fs::read_to_string("/proc/version")
        .ok()
        .map(|s| s.split_whitespace().take(3).collect::<Vec<_>>().join(" "));
    let rustc = std::process::Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_owned());
    json!({
        "cpu_model": cpu,
        "logical_cores": std::thread::available_parallelism().map(usize::from).ok(),
        "mem_total_kib": mem_kib,
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "kernel": kernel,
        "rustc": rustc,
        "build_profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "ci": std::env::var_os("CI").is_some(),
    })
}

/// Onde o relatório vai: `$CAPIA_PERF_OUT` ou `<workspace>/target/perf/<file>`.
pub fn perf_out_path(file: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os("CAPIA_PERF_OUT") {
        return PathBuf::from(dir).join(file);
    }
    let target = std::env::var_os("CARGO_TARGET_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"),
        PathBuf::from,
    );
    target.join("perf").join(file)
}

/// Relatório acumulado de uma suíte de benchmarks.
#[derive(Debug)]
pub struct Report {
    suite: String,
    metrics: BTreeMap<String, Value>,
    extra: BTreeMap<String, Value>,
}

impl Report {
    pub fn new(suite: &str) -> Self {
        Self {
            suite: suite.to_owned(),
            metrics: BTreeMap::new(),
            extra: BTreeMap::new(),
        }
    }

    /// Registra uma métrica de tempo (ms) com as amostras brutas.
    pub fn time(&mut self, name: &str, samples_ms: &[f64], note: &str) -> Stat {
        let st = Stat::of(samples_ms);
        self.metrics.insert(
            name.to_owned(),
            json!({
                "unit": "ms", "n": st.n, "min": st.min, "p50": st.p50, "p95": st.p95,
                "max": st.max, "mean": st.mean, "note": note, "skipped": null,
                "samples": samples_ms,
            }),
        );
        eprintln!(
            "PERF {name}: n={} p50={:.3} ms p95={:.3} ms max={:.3} ms ({note})",
            st.n, st.p50, st.p95, st.max
        );
        st
    }

    /// Métrica não medida **com motivo** (o gate falha se o motivo não estiver na lista permitida).
    pub fn skip(&mut self, name: &str, reason: &str) {
        self.metrics
            .insert(name.to_owned(), json!({ "unit": "ms", "skipped": reason }));
        eprintln!("PERF {name}: SKIPPED ({reason})");
    }

    /// Valor escalar (bytes, contagens): informativo, não é comparado pelo gate de tempo.
    pub fn scalar(&mut self, name: &str, value: Value) {
        self.extra.insert(name.to_owned(), value);
    }

    pub fn to_json(&self) -> Value {
        json!({
            "schema": "capia-perf/1",
            "suite": self.suite,
            "generated_unix_ms": std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| u64::try_from(d.as_millis()).unwrap_or(0))
                .unwrap_or(0),
            "machine": machine_info(),
            "metrics": self.metrics,
            "extra": self.extra,
        })
    }

    /// Grava o JSON em `file` (cria diretórios). Devolve o caminho.
    pub fn write(&self, file: &str) -> std::io::Result<PathBuf> {
        let path = perf_out_path(file);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = serde_json::to_string_pretty(&self.to_json()).map_err(std::io::Error::other)?;
        std::fs::write(&path, text)?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    #[test]
    fn percentiles_use_nearest_rank_and_are_monotonic() {
        let s: Vec<f64> = (1..=100).map(f64::from).collect();
        let st = Stat::of(&s);
        assert_eq!(
            (st.n, st.min, st.p50, st.p95, st.max),
            (100, 1.0, 50.0, 95.0, 100.0)
        );
        let one = Stat::of(&[7.0]);
        assert_eq!((one.p50, one.p95, one.max), (7.0, 7.0, 7.0));
        let unsorted = Stat::of(&[9.0, 1.0, 5.0, 3.0, 7.0]);
        assert_eq!((unsorted.p50, unsorted.p95, unsorted.max), (5.0, 9.0, 9.0));
    }

    #[test]
    fn the_report_keeps_raw_samples_and_skip_reasons() {
        let mut r = Report::new("t");
        r.time("a", &[1.0, 2.0, 3.0], "note");
        r.skip("b", "no ffmpeg");
        let j = r.to_json();
        assert_eq!(j["metrics"]["a"]["samples"].as_array().unwrap().len(), 3);
        assert_eq!(j["metrics"]["a"]["p50"], 2.0);
        assert_eq!(j["metrics"]["b"]["skipped"], "no ffmpeg");
        assert!(j["machine"]["os"].is_string());
    }
}
