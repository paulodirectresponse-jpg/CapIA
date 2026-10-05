use serde::Serialize;

/// Erro da fixture: sempre estruturado em texto (a fixture só roda em testes/benchmarks).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FixtureError(pub String);

impl core::fmt::Display for FixtureError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.0)
    }
}

impl core::error::Error for FixtureError {}

impl FixtureError {
    pub(crate) fn of(what: &str, e: impl core::fmt::Debug) -> Self {
        Self(format!("{what}: {e:?}"))
    }
}

/// Parâmetros do projeto grande. Tudo é determinístico por `seed`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LargeSpec {
    pub seed: u64,
    /// Número de sequences (≥ 2). A de índice 0 é a "main" (a maior; compõe as demais por nested).
    pub sequences: usize,
    /// Total de clips (incluindo nested, texto, legendas e áudio).
    pub clips: usize,
    /// Assets lógicos no documento **e** registros sintéticos (offline) no catálogo.
    pub assets: usize,
    /// AI Runs persistidas (`ai_runs` + stages/eventos/efeitos/orçamento/memória).
    pub runs: usize,
    /// Transações pequenas no fim (histórico realista, cada uma um commit próprio).
    pub history_tail: usize,
    /// Comandos por transação de construção (a construção usa poucas transações grandes).
    pub batch: usize,
}

impl Default for LargeSpec {
    /// Alvo da Fase 6: 36 sequences, 6.000 clips, 2.500 assets, 60 Runs, 300 entradas de cauda.
    fn default() -> Self {
        Self {
            seed: 0xC4B1_A006,
            sequences: 36,
            clips: 6_000,
            assets: 2_500,
            runs: 60,
            history_tail: 300,
            batch: 400,
        }
    }
}

impl LargeSpec {
    /// Versão pequena para testes rápidos (mesma estrutura, escala reduzida).
    pub fn small() -> Self {
        Self {
            seed: 7,
            sequences: 6,
            clips: 240,
            assets: 60,
            runs: 10,
            history_tail: 12,
            batch: 100,
        }
    }
}

/// O que foi realmente construído (medido no arquivo final, não o planejado).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LargeStats {
    pub seed: u64,
    pub sequences: usize,
    pub clips: usize,
    pub nested_clips: usize,
    pub text_clips: usize,
    pub caption_clips: usize,
    pub audio_clips: usize,
    pub media_clips: usize,
    pub keyframed_clips: usize,
    pub markers: usize,
    pub document_assets: usize,
    pub catalog_assets: u64,
    pub history_entries: u64,
    pub revision: u64,
    pub runs: usize,
    pub run_stages: usize,
    pub run_events: usize,
    pub run_effects: usize,
    pub biggest_sequence: String,
    pub biggest_sequence_clips: usize,
    pub biggest_sequence_duration_ticks: i64,
    pub file_bytes: u64,
    pub build_ms: u64,
    /// `document_digest` do documento final: igual para a mesma `LargeSpec` (determinismo).
    pub document_digest: String,
}
