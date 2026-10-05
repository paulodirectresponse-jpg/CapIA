# update

Atualização assinada entre dois builds RC e **rollback** (instalação anterior recuperável).

`node tools/phase6-acceptance/update/run.mjs` — passos em [`steps.json`](steps.json): autoteste do validador (`passed`), testes do atualizador do desktop (`not_available` até existir `updater.rs` + `CAPIA_P6_HEAVY=1`) e a evidência **externa** `signed-update-and-rollback`.

## Evidência (humano/hardware; certificado real para a parte assinada)

`target/phase6-acceptance/evidence/update-results.json`, formato de [`template.json`](template.json), validado por `node tools/phase6-acceptance/update/validate.mjs`.

1. **Atualização assinada** `0.6.0-rc.1 → 0.6.0-rc.2` (dois RCs; `to > from`; sem downgrade): assinatura do manifesto **e** do artefato verificadas, hashes, versão final correta, o projeto abre depois, configurações e referências de segredo preservadas, projetos do usuário intactos.
2. **Rollback**, cada cenário executado de verdade e com `result: "recovered"`, a versão anterior abrindo e os projetos intactos:
   - `interrupted_download` — download interrompido;
   - `corrupt_artifact` — artefato corrompido;
   - `invalid_signature` — update com assinatura inválida/adulterada precisa ser **recusado** (`update_refused: true`);
   - `startup_health_failure` — falha de saúde na inicialização após o update;
   - `active_run` — update com AI Run ativa: `deferred` ou `checkpointed`, nunca corromper o projeto.
3. `downgrade_blocked: true` — sem downgrade acidental.

Sem certificado, registre `signed_update: { "performed": false, "reason": "…" }`: com todos os cenários de rollback recuperados o resultado é `partial` (o gate de update **assinado** segue externo). Cenário falho ou não executado ⇒ `rejected`. Sem arquivo ⇒ `pending_external`.
