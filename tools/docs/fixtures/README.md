# Fixture do catálogo

`catalog.json` é a transcrição **única e mecânica** de `crates/capia-server/src/catalog.rs` (58 operações,
11 scopes, 9 tipos de evento) no estado do commit `2814a93` (branch `claude/phase6-finalization`). Foi gerada por um programa Rust descartável
(um `main` que inclui `scope.rs` e `catalog.rs` por `#[path]` e imprime `catalog::ops()` em JSON) — **não** por
regex sobre o Rust, e o programa não faz parte do repositório.

Formato: `{ version, scopes[], events[], operations[{ name, summary, scope|null, mutating, class, method, path,
status, project, surface: "both"|"rest_only", tool, schema }] }`.

O servidor já expõe `capia-server catalog` (JSON em stdout); para regenerar a partir do binário real (em vez do fixture):

```
capia-server catalog | node tools/docs/gen-api-docs.mjs --catalog -
```

`node tools/docs/gen-api-docs.mjs --check` (CI) falha se `docs/api/{rest-reference.md,openapi.json}` ou o bloco
de scopes de `docs/api/auth-and-scopes.md` não corresponderem ao catálogo informado.
