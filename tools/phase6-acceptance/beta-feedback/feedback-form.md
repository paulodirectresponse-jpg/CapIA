# Formulário de feedback do beta (CapIA 0.6.0-rc.1)

Obrigado por testar. Preencha um formulário **por problema** (e um resumo geral no fim). Não inclua chaves de API, tokens nem mídia de terceiros. Para anexar logs, use **Configurações → Copiar diagnóstico** (versões e tempos, sem caminhos de arquivo) — veja `docs/user/07-solucao-de-problemas.md`.

## Problema

- **Versão do CapIA** (Sobre): ______
- **Windows** (ex.: Windows 11 23H2): ______
- **Em qual etapa aconteceu?** instalar · primeiro projeto · importar · editar · configurar IA · AI Run · aprovações · exportar · API/MCP/webhook · atualizar · desinstalar · outra
- **Resumo (uma frase):** ______
- **Passos para reproduzir:** 1) … 2) … 3) …
- **O que esperava / o que aconteceu:** ______
- **Frequência:** sempre · quase sempre · às vezes · raro · uma vez · não consegui reproduzir
- **Gravidade (sua opinião; a equipe confirma):**
  - **Blocker** — impede usar o produto (não instala, não abre, perde projeto).
  - **Critical** — falha grave sem contorno (export quebra, Run corrompe a timeline, vazamento de dado).
  - **Major** — falha importante com contorno trabalhoso.
  - **Minor** — incômodo com contorno simples.
  - **Cosmetic** — texto, alinhamento, ícone.
- **Diagnóstico anexado?** sim (id do pacote: ______) · não (motivo: ______)

## Resumo geral

- Consegui completar o fluxo **instalar → projeto → importar → editar → exportar**? sim / não
- Consegui configurar IA e rodar uma **AI Run**? sim / não / não tentei
- **Nota geral da experiência** (1 = péssima, 5 = excelente): ___
- O que mais atrapalhou: ______
- O que mais gostou: ______

> Os formulários são registrados no formato de `template.json` (um `participant` por pessoa, um `report` por problema) e validados por `node tools/phase6-acceptance/beta-feedback/validate.mjs --file <arquivo>`.
