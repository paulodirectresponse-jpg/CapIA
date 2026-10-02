# STATUS

**Última atualização:** 2026-10-02 · **Fase atual:** FASE 1 — Fundação (em andamento)

## Estado

| Item | Estado |
|---|---|
| Código de produto | Nenhum (intencional) |
| Arquitetura e documentação | ✅ Concluída (M01); **não alterada** por M02 |
| Auditoria open-source | ✅ Concluída (M02) — `docs/OPEN_SOURCE_AUDIT.md` |
| Estratégia de reuso | **Recomendada: C** (base própria + componentes selecionados), **aguardando aprovação** |
| Decisões abertas | OD-1, OD-2, OD-3 (`DECISIONS.md` §C) + 5 itens de `OPEN_SOURCE_AUDIT.md` §12 |
| ADRs `Proposed` | ADR-016 (core em WASM na UI) — depende do spike S4. Emendas propostas (idempotência, preview→apply) ainda **sem ADR** (`OPEN_SOURCE_AUDIT.md` §9) |
| Spikes técnicos | Não iniciados (S1–S5; propostos S6, S7) |
| Scaffold / CI | Não iniciado |
| Git | Commits locais em `claude/happy-bardeen-feyypa`; **push bloqueado (403)** desde M01 — ver nota abaixo |

## Missões

| Missão | Fase | Resultado |
|---|---|---|
| M01 — Fundação arquitetural do editor AI-first | 1 | ✅ `CLAUDE.md`, 15 documentos em `docs/`, `.gitignore` |
| M02 — Auditoria de bases open-source e estratégia de reuso | 1 | ✅ `docs/OPEN_SOURCE_AUDIT.md`. Conclusão: **nenhum fork**; extrair como referência/semente (OpenCut classic: regras de timeline e compositor wgpu; MartinDelophy: contrato de comandos/MCP; OpenCut novo: build FFmpeg LGPL). Proibidos: tjameswilliams (PolyForm NC), Remotion, vanta |

## Próxima missão recomendada

**M03 — Decisões abertas + spikes técnicos** (renumerada; era "M02" no `ROADMAP.md`/M01):
1. Dono do produto decide: estratégia C, emendas §9 do audit, OD-2 (licença/FFmpeg), OD-3 (hardware de referência).
2. Spikes S1–S5 (`ROADMAP.md` Fase 1) + **S6** (semente de compositor wgpu do OpenCut classic) + **S7** (suíte de aceitação derivada das regras de timeline do OpenCut classic, sem copiar código de produção). Relatórios em `docs/spikes/`.
3. Converter OD-1, ADR-016 e as emendas aprovadas em ADRs `Accepted`.

Depois: scaffold do repositório e CI. Só então a Fase 2 pode começar.

## Notas

- Leia `CLAUDE.md` e este arquivo primeiro; depois os docs relevantes à missão.
- Nenhum código de terceiros foi copiado para o repositório. Qualquer reuso futuro segue as regras de proveniência/clean-room de `OPEN_SOURCE_AUDIT.md` §7.
- **Push:** o commit de M01 (`f01d418`) e o de M02 estão só neste container até o acesso de escrita ao GitHub ser corrigido (https://claude.ai/connect-github).
- Ao concluir uma missão, atualize esta página.
