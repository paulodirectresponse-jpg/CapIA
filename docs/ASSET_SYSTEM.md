# ASSET SYSTEM — Bibliotecas, identidade de mídia, Asset Gateway, mídia gerada

## 1. Camadas de identidade

```
Clip ──AssetRef──► Asset (lógico, nomeado, tags, licença)
                     └── AssetVersion n (proveniência)  ──► MediaFile (físico, fingerprint)
                                                              └── Representations (proxy, thumbs, waveform, frame index, transcript, analysis)
```

- **Asset**: o que o usuário vê na biblioteca ("Produto frasco frente", "SFX whoosh 03"). Tem versões.
- **AssetVersion**: uma realização concreta (import original, download, geração v1/v2...). `Asset.active_version` define o que clips com `VersionPolicy::Active` usam.
- **MediaFile**: arquivo físico, identificado por **fingerprint de conteúdo**, nunca por caminho. Um MediaFile pode servir vários assets/versões/projetos.
- **Representations**: derivados regeneráveis, no cache, chaveados por fingerprint (compartilhados entre projetos).

Um arquivo físico usado em 50 clips de 10 sequences existe **uma** vez: clips referenciam o asset, não copiam nada (ADR-017).

## 2. Kinds

`Video`, `Image`, `Audio`, `Music`, `Sfx`, `Avatar`, `Product`, `Reference`, `Broll`, `Document` (briefs: DOCX/PDF/TXT), `Font`, `Lut`, `Generated` (flag de origem, não exclusivo: um B-roll gerado é `Broll` + provenance de geração). Kind + tags + análise alimentam busca da IA.

## 3. Global Library × Project Library

| | Global Library | Project Library |
|---|---|---|
| Onde | índice em `app.db`; arquivos onde o usuário escolher ou pasta gerenciada `%USERPROFILE%\CapIA Library\` | tabela `assets` do `.capia`; arquivos no lugar original ou em `Nome.capia-media\` |
| Uso | SFX, músicas, packs de B-roll, fontes, LUTs, avatares, assets recorrentes por cliente | Brutos, gravações, entregas, mídias baixadas/geradas para a demanda |
| Relação | Usar um asset global num projeto cria um **asset de projeto que referencia o asset global** (`global_asset_id`) — o projeto continua abrível se a biblioteca global mudar, porque guarda fingerprint e caminhos | — |

Tags por cliente (`client_id`) na Global Library permitem "assets do cliente X".

## 4. Fingerprint, hashing e deduplicação

- **Fingerprint rápido (no import, síncrono):** `size` + BLAKE3 de blocos (primeiros 1 MiB, 1 MiB central, últimos 1 MiB) + duração/streams do probe. Custo constante, suficiente para dedup e relink prático.
- **Hash completo (job em background):** BLAKE3 do arquivo inteiro → `full_hash`. Usado para dedup forte, verificação de integridade, detecção de arquivo alterado no mesmo caminho.
- Import de arquivo cujo fingerprint já existe → reaproveita o `MediaFile` (e representações em cache); pergunta se deve criar novo asset ou reutilizar o existente.
- Arquivo alterado no mesmo caminho (fingerprint diferente) → é **outro** MediaFile; clips antigos ficam offline até decisão do usuário (nunca troca silenciosa de conteúdo).

## 5. Relink e mídia offline

- Na abertura e periodicamente, verifica caminhos (relativo ao projeto → absoluto → caminhos conhecidos).
- Status: `Online`, `Offline` (caminho inacessível, ex.: HD externo), `Missing` (não encontrado após busca), `Changed` (fingerprint não confere).
- **Relink:** usuário aponta uma pasta → busca por fingerprint (rápido) e, como fallback, por nome+tamanho+duração; relink em lote. Também via comando `relink_media` (desfazível).
- **Clips offline:** continuam no documento intactos; preview/render mostram placeholder "Mídia offline" com nome; se houver proxy em cache, preview pode usá-lo (sinalizado) mas **export bloqueia** (ou exige confirmação explícita para exportar com proxy).
- Edição de clips offline é permitida (usando a duração conhecida).

## 6. Asset Gateway

```
Editor / IA (tools gateway.*)
      ↓
Asset Gateway (API estável, jobs, cache, políticas, proveniência)
      ↓
Provider Adapter (trait)                      ← substituível sem tocar no editor
      ↓
Serviço externo (API oficial, serviço próprio, downloader, servidor remoto, browser automatizado)
```

```rust
#[async_trait]
trait GatewayAdapter {
  fn id(&self) -> AdapterId; fn capabilities(&self) -> GatewayCaps; // Fetch, Search, Comments, Metadata
  fn handles(&self, url: &Url) -> bool;
  async fn fetch(&self, req: FetchRequest, ctx: AdapterCtx) -> Result<FetchResult, GatewayError>;     // → arquivos em staging
  async fn search(&self, req: SearchRequest, ctx) -> Result<Vec<SearchHit>, GatewayError>;
  async fn fetch_comments(&self, req: CommentsRequest, ctx) -> Result<Vec<Comment>, GatewayError>;
  async fn health(&self, ctx) -> AdapterHealth;
}
```

Regras:
1. API pública estável: `media.fetch(url)`, `media.search(query, filters)`, `media.fetch_comments(url)` — tudo como **jobs**.
2. Seleção de adapter por `handles(url)` + prioridade configurada + saúde; vários adapters para o mesmo domínio permitem fallback.
3. Adapters que dependem de ferramentas externas (ex.: downloaders) rodam como **sidecar/processo separado**, com versão própria e atualização independente; um adapter quebrado vira `health=Down` e o app segue funcionando.
4. Saídas passam por **staging** → validação (probe, tamanho, tipo) → import normal (fingerprint, asset, proveniência). Adapter nunca escreve no projeto.
5. Credenciais de adapters (tokens de API) seguem o mesmo `SecretStore` (`SECURITY.md`).
6. Proveniência obrigatória: URL de origem, adapter+versão, data, metadados do autor, **licença/termos declarados** (`LicenseInfo`), e flag de uso ("referência apenas" vs "utilizável no criativo").
7. Riscos legais/ToS: o app não embute downloaders por padrão; adapters de terceiros são opcionais e configurados pelo usuário; materiais marcados "referência apenas" não podem ser inseridos em timelines de deliverables sem confirmação (warning de validação, não bloqueio técnico).

## 7. Mídia gerada por IA

```rust
struct Provenance {
  source: Import{original_path} | Download{url, adapter, version, retrieved_at, license}
        | Generated{ provider, model, model_version, prompt, negative_prompt, seed: Option<u64>, params: Json,
                     input_assets: Vec<AssetVersionId>, cost: Option<Money>, run_id: Option<RunId>, created_at }
        | Derived{ from: AssetVersionId, operation }   // ex.: CFR intermediate, crop exportado
}
```

- **Regenerar** = nova `AssetVersion` do mesmo Asset (mesmo prompt/parâmetros ou editados). Clips com `VersionPolicy::Active` passam a usar a nova versão após um comando `set_active_version` (desfazível); clips `Pinned` não mudam.
- **Substituir** um asset por outro em todos os clips: comando `replace_asset_refs` (desfazível).
- Se a nova versão tiver duração menor que o range usado por algum clip, a validação aponta o conflito antes de ativar (opções: estender com freeze, ajustar clip, cancelar).
- Arquivos gerados são baixados imediatamente para `Nome.capia-media\generated\` (URLs de provider expiram).
- Custos agregados por projeto.

## 8. Análises e metadados reutilizáveis

Por fingerprint (portanto compartilhado entre projetos): probe, frame index, transcript (com idioma/modelo), detecção de cenas, descrições de frames (com modelo e data), análise de referência. Cada análise guarda `producer` (modelo/algoritmo + versão) para invalidação seletiva.

## 9. Import

Arrastar arquivos/pastas → probe síncrono leve → asset criado imediatamente (utilizável) → jobs: full hash, frame index, thumbnails, waveform, proxy (se necessário), transcrição (se configurado auto). A timeline nunca espera jobs para permitir edição.

## 10. O que a M07 implementou (e onde diverge deste documento)

Implementado: identidade por conteúdo, hash em streaming, dedup (um asset por conteúdo por projeto), catálogo no `.capia` (schema 2), import atômico, online/offline/modified, relink por conteúdo, `verify`, cache `CacheKey`/`CacheDir` e miniatura (ADR-046..049).
**Divergências conscientes:** (1) hash = **SHA-256 do arquivo inteiro no import** (sem fingerprint BLAKE3 amostrado + job de hash completo em background — não há jobs ainda; o prefixo `sha256:` deixa a troca como migração futura); (2) o `AssetId` é derivado do hash na criação (`ast_<32 hex>`), não há `AssetVersion`/`MediaFile` separados ainda; (3) o status `Missing` não existe (só `offline`); (4) *force-relink*, relink em lote por pasta e busca por nome+tamanho ficam para depois. Itens das §3, §6–§8 (Global Library, Asset Gateway, mídia gerada, análises) seguem fora de escopo.

## 11. Fase 5 — Asset Gateway implementado (ADR-093..096)

Implementação em `crates/capia-intelligence/src/autonomy/gateway.rs` e `generation.rs`; download em `crates/capia-ai/src/fetch.rs`. Onde diverge do §6/§7 acima, vale esta seção.

- **Adapters** (`AssetGatewayAdapter`): `LocalLibrary` (pasta local), `ApprovedUrl` (allow-list de hosts por adapter) e `ReplayCatalog` (testes/dev). Sem adapters de downloader/sidecar nesta fase; adapter desligado ⇒ o app continua, o orquestrador replaneja (need opcional) ou vai a `WAITING_USER` (need crítica; política `on_critical_unavailable`).
- **Needs tipados:** o Brain não busca por URL; o Planner declara `AssetNeed` (finalidade, tipo, ordem de aquisição da política `acquire_order` (padrão `project → library → gateway → generate`)). Candidatos são ranqueados (componentes persistidos) e o metadado externo é **dado não confiável** (nunca instrução, nunca memória).
- **`SafeFetcher`:** `https`, host da allow-list a cada redirect, DNS sem IP privado/link-local/metadata, tipo de conteúdo permitido, teto de bytes durante o stream, hash SHA-256 em streaming, `*.part` + `rename`, cancelamento real, sem credencial seguindo redirect.
- **Licença:** `license_verdict`: `KnownAllowed/UserProvided/Generated` permitem; `Unknown` exige aprovação (padrão); `KnownRestricted` rejeita (padrão) ou pede decisão. Efeitos pagos reservam orçamento antes.
- **Proveniência:** `ai_provenance(asset_id, run_id, kind, content_hash, json)` — adapter, fonte, licença/termos, hash, run; para geração, prompt, modelo, parâmetros e custo. Consulta por `ai.run.provenance`.
- **Mídia durável:** o arquivo adquirido/gerado vai de staging (`<projeto>.capia-cache/autonomy/staging`) para `<projeto>-media/ai/<sha>.bin` (**durável**; o cache é descartável e o catálogo guarda o caminho), e entra pelo sistema de assets (hash + probe + documento + catálogo na MESMA transação; `EditorApi::agent_import_*`, só Rust) sob atribuição da Run (efeito `imp:*`). Falha nunca deixa asset parcial válido (a identidade é o conteúdo, ADR-046). Versões novas de mídia gerada nunca sobrescrevem a anterior.
- **Geração:** opt-in (desligada por padrão), com aprovação e orçamento; `job_id` persistido e consultado antes de novo submit.
- **Limpeza:** `ai.run.cleanup` lista candidatos (assets da Run); remover é ação do usuário, não da IA.
