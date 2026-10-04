# Tarefa de aceitação — anúncio UGC de 30–45 s

Objetivo: provar que uma pessoa **sem treinamento** consegue montar um anúncio curto no CapIA
**só com edição manual** (sem IA). Cada participante faz a tarefa uma vez, cronometrada, e relata
travas e confusões. **Não ajude** durante a tarefa; anote o que você teria dito.

## Preparação (facilitador, 5 min)

1. `node make-sample.mjs sample-media` (gera talking head, 3 B-rolls, música, SFX e logo).
2. Abra o CapIA. Idioma à escolha do participante (Configurações → Idioma).
3. Abra `timer.html` num navegador ao lado. Clique **Iniciar** quando o participante receber o roteiro.

## Roteiro entregue ao participante

> Você vai montar um anúncio vertical de 30 a 45 segundos para um produto fictício.
> Arquivos em `sample-media`. Salve o projeto como `anuncio.capia` e exporte `anuncio.mp4`.

| # | Passo | Critério (o facilitador marca no `timer.html`) |
|---|---|---|
| 1 | Criar o projeto e importar **toda** a pasta `sample-media` | os 6 arquivos aparecem na biblioteca |
| 2 | Colocar o **talking head** na timeline e **cortar** o início e o fim (trim) deixando ~30–40 s | clip com duração 30–45 s |
| 3 | **Dividir** (split) o talking head em 2 pontos e **apagar** um trecho do meio (ripple delete) | ≥ 1 split + ≥ 1 delete |
| 4 | Inserir **2 ou 3 B-rolls** por cima (faixa Overlay) em momentos diferentes | ≥ 2 B-rolls visíveis no preview |
| 5 | Adicionar **uma transição** entre dois clips | transição visível no inspector |
| 6 | Adicionar **um título** (texto) nos primeiros segundos | texto aparece no preview |
| 7 | Adicionar **legendas manuais** (≥ 3 linhas) na faixa de legendas | 3 legendas na lista |
| 8 | Colocar a **música** e baixar o volume para ~ −18 dB; **SFX** no momento de um corte | volume alterado; SFX na timeline |
| 9 | **Desfazer** e **refazer** uma ação (qualquer) | uso do histórico/atalho |
| 10 | **Exportar MP4** (H.264) | arquivo gerado e validado pelo app |

Tempo máximo: 45 min. Parou de avançar por 3 min? Anote como **trava**, dê a dica mínima e siga.

## Medidas (preencher em `results-template.json`)

- Tempo total e por passo (o `timer.html` exporta o JSON).
- Bugs que **impediram** de continuar (bloqueantes) e bugs menores — severidade em `ISSUE-FORM.md`.
- Pontos de confusão relevantes (onde hesitou, onde clicou errado, o que não achou).
- Diagnóstico não sensível: Configurações → **Copiar diagnóstico** (cole em `diagnostics`).

## Critério (ROADMAP, caixa humana)

≥ 3 pessoas distintas completam os 10 passos (quem edita vídeo com frequência, em **≤ 15 min** — o
ROADMAP fala em editor experiente); **zero bugs bloqueantes** não resolvidos; tempo e confusões registrados. Quem completar menos de 10 passos entra como *resultado real*, não é descartado.
