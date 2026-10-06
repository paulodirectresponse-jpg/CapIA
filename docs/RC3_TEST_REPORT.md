# RC3 (0.6.0-rc.3) — relatório de testes

Escopo: fechar os pontos da auditoria externa do RC2. **Este documento só afirma o que foi executado.** O que depende de pessoa, chave, certificado ou hardware está em "Pendências externas" e **não** foi marcado como feito.

## Comprovação em CI (branch `claude/rc3-free-timeline-ux`)

| Evidência | Resultado |
|---|---|
| CI completo (6 jobs: Núcleo Rust Linux, Rust+desktop Windows, TypeScript, Arquitetura/licenças/segredos, E2E Linux, **E2E Windows no app Tauri/WebView2 real**) no commit de código `fe117f7` | **verde — run 37461493186** |
| Runs anteriores | 37454500011 (`5991c63`): 5/6 verdes; o E2E Windows achou que o spec `free-timeline` arrastava para x além da janela nativa — corrigido em `fe117f7`. 37450630427 (`f6d5017`): falhas do harness (código de saída) e do teste de rajada de conexões — corrigidas. |
| CI e instalador no **HEAD final** (que difere de `fe117f7` só por este relatório) e SHA256/artefato | informados na resposta final da entrega (não cabem aqui sem referenciar o próprio commit) |

## 1. Executor de jobs (causa real do run 136)
`the_sink_sees_every_transition_in_order` falhou no Windows (`[Running, Completed, Completed]`). O snapshot "Queued" era tirado depois de o job estar visível aos workers. Correção na produção (mutex de ordenação por job, ADR-121); teste determinístico `queued_is_always_delivered_before_running_even_when_the_sink_is_slow` (falha no código antigo, passa no novo); o teste original ficou intacto.

## 2. Timeline livre (E2E só por interface)
`packages/e2e/tests/free-timeline.spec.ts`: projeto sem tracks → importa 3 vídeos + 4 áudios → arrasta da biblioteca (vídeo 1 cria a primeira camada; 2 e 3 criam novas tracks acima; áudio 1 e 2 criam tracks de áudio; áudio 3 e 4 coexistem) → move clipes entre tracks e para o espaço vazio (cria track) → renomeia (duplo clique), move para cima/baixo (menu), trava, mudo, exclui track vazia → fecha e reabre pela Home (estrutura preservada) → exporta. A API do engine só **lê**. Passa no Linux (Chromium) e **no app Windows real**. Novos comandos `rename_track`/`move_track` (ADR-122) e salto de track em `resolve_group_move` com teste de núcleo. Os specs antigos que dependem de papéis semeiam tracks por API (preparação de teste).

## 3–5. Redesign, texto/legendas, sequências
`rc3-ux.spec.ts` (Linux e Windows): rail com exatamente Mídia/Texto/Áudio/Transições/IA; inspector sem seleção simples com Avançado recolhido; sem dados técnicos no preview; Adicionar texto/título/legenda, edição, entrada pronta (Aparecer) e desfazer; Sequências dentro de Mídia (lista, dica, "+ Nova sequência", arrastar para usar, dois cliques entra, migalhas voltam, sem "nested"); legibilidade sem rolagem da página e controles dentro da janela em 1366×768, 1536×864 (125%), 1280×720 (150%) e 1093×614 (1366 a 125%). Capturas: `target/e2e-results/rc3-scale-*.png` (artefato do job). **A aprovação visual humana nessas escalas continua pendente.**

## 6. Export medido (1080p, mídia real, 4 núcleos, `mpeg4-reference`)
`cargo test --release -p capia-project --test perf_export -- --ignored --nocapture` (JSON em `target/rc3-export-bench.json`). Etapas separadas: áudio, partida do encoder, decode, composição, espera do encoder, drenagem/mux, validação, `fsync`.

| Cenário (quadros) | Antes (serial, ×tempo real / fps) | Depois | Onde estava o tempo (antes → depois) |
|---|---|---|---|
| 3 s, 1 camada (90) | 1,04× / 35 fps | 1,46× / 54 fps | composição 1147 → 942 ms |
| 3 s, 2 camadas (90) | 0,52× / 17 fps | 0,97× / 33 fps | composição 3351 → 2087 ms |
| 30 s, 2 camadas (900) | 0,62× / 19 fps (48,6 s) | **1,20× / 37 fps (25,0 s)** | composição 34,1 → 20,2 s; escrita do encoder (9,6 s, serializada) → espera ≈ 0 |

Gargalo evitável corrigido sem mudar a saída: compositor em faixas de linhas (idêntico bit a bit ao serial — teste `threaded_blit_is_bit_identical_to_serial`) e render em paralelo com a escrita do encoder. Decode ≈ 4 s de 25 s; o gargalo restante é a composição por CPU (ADR-124). Não foi medido com H.264 de hardware.

## 7–8. Chat e OpenAI real
- **E2E de chat pela interface** (`ai.spec.ts`, cérebro Replay determinístico — não é OpenAI real): clipe selecionado por clique → "remova os primeiros 2 segundos deste clipe" → proposta aparece sem alterar o documento → aprovar → duração −2 s e largura do clipe na timeline diminui → desfazer volta **exatamente** ao JSON original.
- **`tools/rc3-acceptance/openai-real.ps1`**: usa a credencial já guardada no Credential Manager (lida só dentro do Rust, via `ai.connect use_stored` e `capia-devserver --os-vault`), nunca a imprime, varre os logs por padrão de chave. **Executado no CI Windows sem chave: todos os passos `pending_external`, exit 3 — nunca sucesso.** O fluxo real (conectar, chat, edição com aprovação e desfazer, visão, `whisper-1`) **não foi executado com a chave real**: é o gate externo. Artefato `capia-rc3-live-harness` no workflow do instalador.

## 9. Erros que não matam a WebView
`ai.spec.ts` (Linux): falha do provedor no chat, falha de transcrição, cancelamento de resposta em andamento e erro inesperado do engine (HTTP 500/JSON inválido) — nenhum mostra a tela de falha, o editor segue editável. Achado e corrigido: a falha do provedor no chat ficava **muda** (a UI só tratava a fase `error`; o assistente devolve registro `failed`). O Error Boundary global segue coberto por `ErrorBoundary.test.tsx`.

## Compatibilidade com RC1/RC2
Um projeto criado pelo **binário do RC2** (commit `1026233`: tracks de papel Main/Overlay/Voice, texto, keyframes, dissolução) foi aberto pelo binário do RC3: `inspect`/`validate` ok, `dump` do documento **idêntico**, `render frame` em 7 instantes (inclusive na transição) com **o mesmo SHA-256 dos quadros** nos dois binários, e `rename_track`+`move_track`+`undo` no RC3 devolveram o documento ao estado do RC2 (igual, exceto o contador de revisão). Execução manual (CLI), não automatizada em CI.

## Pendências externas (abertas, nunca marcadas)
OpenAI real com a chave do usuário (harness entregue), certificado de assinatura (o instalador é candidato **não assinado**), Windows 10/11 limpos, beta com usuários reais, decisão jurídica de H.264/AAC, pentest independente, endpoint de crash, aprovação visual humana em 125%/150%.
