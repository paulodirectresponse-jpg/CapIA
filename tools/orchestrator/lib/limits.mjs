// Hard stops: qualquer limite estourado interrompe o laço ANTES do próximo passo.
export function evaluateLimits({ limits, state, sessionWallMs }) {
  const c = state.counters;
  const wallMs = c.wallMs + sessionWallMs;
  if (state.iteration > limits.maxIterations) return `limit:maxIterations(${limits.maxIterations})`;
  if (wallMs >= limits.maxWallClockMinutes * 60_000)
    return `limit:maxWallClockMinutes(${limits.maxWallClockMinutes})`;
  if (c.costUsd >= limits.maxCostUsd)
    return `limit:maxCostUsd(${limits.maxCostUsd}) gasto=${c.costUsd.toFixed(2)}`;
  if (c.claudeCalls >= limits.maxClaudeCalls)
    return `limit:maxClaudeCalls(${limits.maxClaudeCalls})`;
  if (c.codexCalls >= limits.maxCodexCalls) return `limit:maxCodexCalls(${limits.maxCodexCalls})`;
  if (c.consecutiveFailures >= limits.maxConsecutiveFailures)
    return `limit:maxConsecutiveFailures(${limits.maxConsecutiveFailures})`;
  if (c.noProgress >= limits.maxNoProgress)
    return `no-progress:${c.noProgress} auditorias seguidas com os mesmos achados`;
  return null;
}
