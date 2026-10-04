# Checklist do facilitador

Antes: [ ] CapIA instalado/executável  [ ] ffmpeg/ffprobe disponíveis  [ ] `make-sample.mjs` rodou
[ ] `timer.html` aberto  [ ] gravação de tela (opcional)  [ ] participante não viu o app antes.

Durante: [ ] não ajudar  [ ] marcar cada passo no timer  [ ] anotar travas/confusões  [ ] anotar bugs (ISSUE-FORM).

Depois: [ ] export validado (o app mostra codec/resolução)  [ ] **Copiar diagnóstico** colado
[ ] `results-template.json` preenchido (cópia por participante)  [ ] `node validate-results.mjs results/*.json`.

Residual de GPU (uma vez, em PC com GPU real): `powershell -ExecutionPolicy Bypass -File .\gpu-residual.ps1`
[ ] saída PASS  [ ] ou FAIL com o `gpu-residual-result.json` anexado (gatilho de reabertura da ADR-069).
