# Problemas conhecidos e pendências externas — 0.6.0-rc.2

Derivado de `docs/STATUS.md` (seções das Fases 3, 4 e 5) e dos documentos da Fase 6. **“Pendente externo”** = depende de máquina, pessoa, provedor, certificado ou decisão que a engenharia não tem; nunca é marcado como feito sem evidência real (validadores em `tools/phase*-acceptance/`).

## Gates externos da Fase 6 (bloqueiam `PHASE 6 COMPLETE`)

| # | Pendência | Como é fechada | Harness |
|---|---|---|---|
| E1 | **Certificado de assinatura de código real** (instalador, atualizador, executáveis) | adquirir, assinar o RC, verificar Authenticode com carimbo de tempo | `tools/phase6-acceptance/installer/` |
| E2 | **Windows 10 22H2 e Windows 11 limpos (físicos)**, sem ferramentas de dev: instalar → criar → importar → editar → exportar → desinstalar | executar `run-clean-machine.ps1` nas duas máquinas | `clean-machine/` |
| E3 | **Atualização assinada RC→RC e rollback** em Windows real | executar os cenários e registrar evidência | `update/` |
| E4 | **Beta com usuários reais** (mín. documentado: 5 externos) e gate “sem Blocker/Critical aberto” | coletar feedback real | `beta-feedback/` |
| E5 | **Decisão de produto/jurídica sobre H.264/AAC** (patentes, OpenH264, qualidade de produção) | decisão do Product Owner/jurídico; registrar ADR | — |
| E6 | **Paridade UI × REST × MCP e fluxo canônico contra o servidor real** | executar a mesma tarefa pelas três superfícies | `external-flow/` |
| E7 | **Pentest independente** da API local | contratar/serviço externo | `security/` |
| E8 | Verificação do **pacote de licenças** (FFmpeg LGPL, avisos de terceiros) no instalador final | revisão do artefato | `installer/` |

## Pendências herdadas (Fases 3, 4, 5)

- **Fase 3:** teste cronometrado com ≥ 3 usuários reais (`tools/phase3-acceptance/`); residual de CPU/pacing do preview P2 em **GPU real** (`gpu-residual.ps1`; o runner Windows do CI é software); decisão `OUTPUT-H264`.
- **Fase 4:** qualidade de LLM real no DemandSpec e smoke com chaves reais das 3 famílias; corpus real de vídeos anotados por humanos para detecção de cortes (o corpus sintético não substitui); transcrição real (whisper.cpp) medida em fala real.
- **Fase 5:** avaliação humana de ≥ 10 demandas reais (média ≥ 4,0); providers/chaves reais (Brain, geração, fontes do Gateway); checagem humana no desktop Windows com provider real; qualidade do Critic com **visão real**.

## Limitações do produto

- **Plataforma:** o alvo é Windows. Sem cofre do SO (outras plataformas) as chaves ficam só em memória.
- **H.264:** só encoders **aprovados** (`h264_mf` provado no runner Windows, software); nenhum encoder GPL; sem H.264 aprovado a exportação falha com mensagem clara; na VM Linux não há H.264 aprovado.
- **Gateway:** fontes locais, URL aprovada e catálogo de teste; **sem** integração com banco de stock real.
- **Detecção de cenas** calibrada em material sintético; vídeo real com movimento de câmera/efeitos pesados pode exigir ajuste.
- **Render/preview:** compositor CPU de referência; `render.frame` não é oferecido ao assistente; amostragem de quadros para visão usa FFmpeg e exige modelo de visão e privacidade que permita.
- **Assistente** pontual (≤ 8 passos); a Run autônoma é o caminho para tarefas longas.
- **Migrações só para frente:** projeto de schema mais novo que o app é recusado; não existe downgrade ([`MIGRATION_COMPAT.md`](MIGRATION_COMPAT.md)).
- **Projeto aberto único:** o servidor/engine hospeda um projeto aberto por vez (`NO_PROJECT_OPEN`/`PROJECT_NOT_OPEN` em outro).

## Fase 6 — em integração (ver `docs/phase6/IMPL_DOCS_ACCEPTANCE.md`)

- **MCP stdio** (`capia-server mcp-stdio`): declarado na CLI; no snapshot de integração usado pela documentação ainda “not implemented yet”. Resources `capia://…` e anotações de efeito são **interface prevista**.
- **Entrega de webhooks:** o cliente HTTP endurecido existe e é testado; a documentação do formato exato de cabeçalhos/timestamp e do número de tentativas é **interface prevista** até a frente de webhooks fechar.
- **Instalador, atualizador, pacote de diagnóstico e relatório de falhas opt-in:** frente de desktop/distribuição; os documentos de usuário os descrevem como Fase 6 e a verificação em máquina limpa é externa.
- **Exemplos** foram testados só contra servidores falsos; **não** foram executados contra o `capia-server` real (item E6).
- Documentação **não** foi revisada por um usuário novo seguindo-a do zero (PHASE6_BETA §20: “fresh tester can complete canonical workflow following docs” — externo).
- Acessibilidade (teclado, escala) e **localização** (strings pt-BR/en) não passaram por revisão humana; a paridade de chaves pt-BR/en é verificada por teste.

## Como reportar

Use o formulário em `tools/phase6-acceptance/beta-feedback/feedback-form.md` e anexe o diagnóstico redigido ([`docs/user/07-solucao-de-problemas.md`](user/07-solucao-de-problemas.md)). Nunca anexe chaves, tokens nem mídia de terceiros.
