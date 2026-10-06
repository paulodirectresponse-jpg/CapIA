// Partes puras do runner da suíte de segurança do servidor (Fase 6, Track D-2): leitura da saída do
// `cargo test`, do JSON da mutação e montagem do resumo. Nada aqui executa processo nem inventa
// resultado: o que não rodou ou falhou nunca vira "passou".

/** Soma as linhas `test result: ok. N passed; M failed; …` de uma saída do cargo. */
export function parseCargoSummary(text) {
  const out = { passed: 0, failed: 0, ignored: 0, binaries: 0 };
  for (const m of text.matchAll(
    /test result: (ok|FAILED)\. (\d+) passed; (\d+) failed; (\d+) ignored/g,
  )) {
    out.binaries += 1;
    out.passed += Number(m[2]);
    out.failed += Number(m[3]);
    out.ignored += Number(m[4]);
  }
  return out;
}

/** Nomes dos testes que falharam (`test <nome> ... FAILED`). */
export function failedTests(text) {
  return [...text.matchAll(/^test (\S+) \.\.\. FAILED$/gm)].map((m) => m[1]);
}

/** `[[id, nome, veredito, segundos], …]` (saída de tools/mutation-phase6.py) → resumo. */
export function summarizeMutation(rows) {
  const verdicts = { DETECTED: 0, SURVIVED: 0, "BROKE-BUILD": 0, "PATTERN-NOT-FOUND": 0 };
  const survivors = [];
  for (const [id, name, verdict] of rows) {
    verdicts[verdict] = (verdicts[verdict] ?? 0) + 1;
    if (verdict !== "DETECTED") survivors.push({ id, name, verdict });
  }
  return {
    total: rows.length,
    detected: verdicts.DETECTED,
    verdicts,
    // só vale como "passou" se TODAS foram detectadas (mutante sobrevivente é achado)
    passed: rows.length > 0 && verdicts.DETECTED === rows.length,
    survivors,
  };
}

/** Passo que não terminou (sem código de saída) conta como falha. */
export function stepPassed(status) {
  return status === 0;
}

export function buildSummary({ generated, steps, mutation }) {
  const totals = steps.reduce(
    (a, s) => ({
      passed: a.passed + (s.tests?.passed ?? 0),
      failed: a.failed + (s.tests?.failed ?? 0),
    }),
    { passed: 0, failed: 0 },
  );
  const stepsOk = steps.every((s) => s.passed);
  const mutationOk = mutation === null ? null : mutation.passed;
  return {
    suite: "security-suite",
    generated,
    passed: stepsOk && mutationOk !== false,
    mutation_run: mutation !== null,
    tests: totals,
    steps,
    mutation,
  };
}
