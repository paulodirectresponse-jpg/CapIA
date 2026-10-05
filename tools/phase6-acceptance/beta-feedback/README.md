# beta-feedback

Feedback **real** do beta e o **gate de saída**: *nenhum Blocker/Critical conhecido em aberto para o release candidate*.

> **Status: externo.** Não há usuários de beta reais neste ambiente; nada foi coletado. Sem arquivo, o validador responde `pending_external`. Nunca preencha com feedback inventado.

## Coleta

- Cada participante recebe [`feedback-form.md`](feedback-form.md) (pt-BR). Um problema = um relatório.
- Registre em JSON no formato de [`template.json`](template.json): `participants[]` (pseudônimo, `external_beta_user|internal`, se completou o fluxo canônico, nota 1–5) e `reports[]`:
  - `severity`: **Blocker** · **Critical** · **Major** · **Minor** · **Cosmetic**;
  - `reproducibility`: `always|often|sometimes|rare|once|unable`;
  - `workflow`: `install|first_project|import|edit|ai_setup|run|approvals|export|api|mcp|webhook|update|uninstall|other`;
  - `app_version`, `os`, `logs` (`attached` + `bundle_id`, ou `unavailable_reason`), `rating`, `status` (`open|fixed|wontfix|duplicate|not_a_bug`), `fixed_in`, `justification`…
- Logs: peça o **bundle de diagnóstico redigido** (sem chaves, sem mídia) — ver `docs/user/07-solucao-de-problemas.md`.

## Validação e gate

`node tools/phase6-acceptance/beta-feedback/validate.mjs --file feedback.json [--min-users 5]`

- **Gate:** um Blocker/Critical conta como **aberto para o RC** se `status` é `open`, `wontfix`, ou `fixed` em versão **posterior** ao RC (a correção não está no RC). `wontfix` nunca fecha Blocker/Critical; `not_a_bug` exige justificativa; `duplicate` aponta outro relatório existente; `affects_rc: false` exige motivo. Qualquer bloqueador aberto ⇒ `rejected` (gate `failed`).
- Blocker/Critical exigem passos para reproduzir e logs (ou motivo de indisponibilidade).
- **Beta suficiente:** o mínimo **documentado** é 5 usuários externos reais (`--min-users`; escolha de produto, não vem do ROADMAP); usuários `internal` não contam. Menos que isso ⇒ `partial` (o gate é só informativo); nenhum externo ⇒ `pending_external`.
- Saída inclui contagem por severidade, abertos por severidade, usuários que completaram o fluxo e a nota média.
