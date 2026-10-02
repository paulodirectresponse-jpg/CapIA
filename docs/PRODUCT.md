# PRODUCT — Visão e escopo

## 1. Visão

Um editor de vídeo desktop profissional em que a IA é um **co-editor** que opera o mesmo motor de edição que o usuário. O foco é produção em volume de criativos de performance: **Direct Response, UGC, Ads e VSLs curtas/médias**.

A proposta de valor:

1. **Velocidade de edição manual** comparável ao CapCut Desktop (familiaridade de interação), com identidade visual própria.
2. **Demandas inteiras, não vídeos isolados:** um projeto contém todas as variações (hooks × bodies × formatos) de uma demanda.
3. **IA que entende a demanda antes de editar:** lê briefing/copy/referências, planeja, adquire assets, edita, revisa, corrige — e tudo o que produz continua editável.
4. **Independência de provider:** o usuário escolhe qual modelo é o "cérebro" e quais modelos fazem visão, transcrição, geração de imagem/vídeo.

## 2. Público

| Persona | Necessidade principal |
|---|---|
| Editor de DR/UGC freelancer ou in-house | Produzir dezenas de variações por semana, rápido, com padrões de edição de alta conversão |
| Agência/media buyer | Lotes de criativos por cliente com consistência de estilo (memória por cliente) |
| Produtor de VSL | Edições médias (5–30 min) com legendas, B-roll, ritmo controlado |

## 3. Princípios de produto

1. **Timeline é a fonte de verdade.** A IA não tem um "estado paralelo" do vídeo; ela produz clips comuns.
2. **Funciona offline / sem IA.** Se todos os providers falharem, o editor continua 100% utilizável.
3. **IA não usa computer-use.** IA e usuário compartilham o Command Engine.
4. **Pensar antes de agir:** UNDERSTAND → PLAN → VALIDATE PLAN → ACQUIRE → EDIT → REVIEW → CORRECT → DONE.
5. **Transparência:** toda alteração tem autor (usuário, agente, API), motivo e é desfazível como unidade.
6. **Local-first:** mídia, projeto, cache e preview locais; nuvem apenas para inferência/geração.
7. **Custo explícito:** o usuário vê e limita gasto de IA por execução/projeto.

## 4. Unidade principal: Projeto = Demanda

```
Workspace (instalação local do usuário: bibliotecas globais, providers, memória de usuário/cliente)
└── Project / Demand  ("Nulle S3 - Setembro")
    ├── Brief, Research, Edit Plans (estados de IA, separados da timeline)
    ├── Project Library (assets)
    ├── Folders (organização)
    │   ├── AD 1/  → Hook 1, Hook 2, Hook 3, Body (sequences)
    │   ├── AD 2/  → ...
    │   └── Masters/ → BODY_MASTER (sequence reutilizável)
    └── Deliverables (sequence + preset de export + nome de arquivo)
```

Hierarquia de edição: `Workspace → Project → Sequence → Track → Clip`.
Pastas organizam sequences; **não** são um nível de edição (ver `DECISIONS.md` ADR-006).

## 5. Escopo da V1 (fim da Fase 6)

- Edição multi-sequence com nested sequences, tracks ilimitadas, transições, keyframes, texto, legendas, áudio/música/SFX.
- Preview em tempo real com proxies; export H.264/HEVC em formatos 9:16, 1:1, 4:5, 16:9.
- Reframe básico (fit/fill/blur-fill), zoom/punch-in, transform, crop, opacidade, ajustes de cor básicos.
- Transcrição (local ou cloud) e legendas automáticas com estilo por palavra.
- AI Brain configurável, Capability Router, tool system com permissões.
- Demand Interpreter, **Reference Analyzer (requisito V1)**, Planner, Editor, Critic.
- Asset Gateway com adapters substituíveis; mídia gerada por IA com proveniência e versões.
- Memória (System/User/Client/Project) sem aprendizado global automático.
- Geração de variações (matriz hooks × bodies) usando nested sequences.
- REST API, MCP Server e Webhooks operando a mesma engine.

## 6. Fora do escopo inicial

Color grading profissional, 3D, rotoscopia profissional, compositing estilo Fusion/After Effects, tracking profissional, multicam profissional, centenas de efeitos, colaboração em tempo real, marketplace de plugins. A arquitetura não deve impedir esses itens, mas nenhum esforço é gasto neles.

## 7. Métricas de sucesso do produto (orientativas)

- Editor experiente monta um UGC ad de 30–45 s (talking head + B-roll + legendas + música) sem IA em ≤ 15 min.
- Com IA: brief + bruto + referência → 3 variações editáveis em ≤ 10 min de relógio, custo exibido antes de aprovar.
- Zero perda de trabalho em crash (no máximo a última operação não confirmada).

## 8. Glossário

| Termo | Definição |
|---|---|
| **Workspace** | Escopo local do usuário: config, bibliotecas globais, providers, memória de usuário/cliente |
| **Project / Demand** | Arquivo `.capia`; uma demanda completa com várias sequences |
| **Sequence** | Uma timeline com formato próprio (resolução, fps, áudio) |
| **Track** | Faixa ordenada de uma sequence; família `Visual` ou `Audio`, com `role` opcional |
| **Clip** | Item posicionado numa track (mídia, texto, legenda, nested sequence, sólido) |
| **Nested Sequence / Composition** | Clip que referencia outra sequence do mesmo projeto |
| **Asset** | Entrada lógica de biblioteca (pode ter várias versões) |
| **MediaFile** | Arquivo físico identificado por fingerprint |
| **Ticks** | Unidade inteira de tempo: 1/705.600.000 s |
| **Command** | Intenção estruturada de alteração validada pelo Command Engine |
| **Transaction** | Grupo atômico de comandos (commit/rollback, uma entrada de undo) |
| **Brain** | Modelo primário de orquestração da IA |
| **Brain Profile** | Brain + overrides por capability + orçamentos |
| **Capability Router** | Resolve qual modelo/provider atende cada capability |
| **Demand Spec** | Especificação estruturada da demanda (saída do Demand Interpreter) |
| **Reference Grammar** | Gramática de edição extraída de um vídeo referência |
| **Edit Plan** | Plano estruturado de edição de um deliverable (antes de virar comandos) |
| **Deliverable** | Sequence + preset de export + regra de nome |
| **AI Run** | Uma execução persistida do pipeline de IA |
