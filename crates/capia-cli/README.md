# capia-cli

Binário `capia`: cliente headless do Command Engine sobre arquivos `.capia`. Não duplica parser nem engine (usa `capia-project`).

```
capia create  <arq.capia> [--json]
capia inspect <arq.capia>            capia validate <arq.capia>
capia apply   <arq.capia> <tx.json|-> [--actor user|agent|api] [--label ...]
capia undo|redo <arq.capia>          capia history <arq.capia>
capia dump    <arq.capia> [--pretty]   # JSON determinístico do documento
```

`apply` aceita uma Transaction, uma lista de envelopes ou um envelope (os comandos serializáveis do engine). Atores `agent`/`api` passam por `preview → apply_plan`. Saída: 0 ok · 1 falha (JSON estruturado em stderr) · 2 uso. Testes: `tests/cli.rs` (executa o binário).
