# 2. Manual do editor

O editor é um cliente do **Command Engine**: toda alteração (sua, da IA, da CLI ou da API) é um comando validado, transacional e com desfazer. Tempo é sempre inteiro (quadros), sem arredondamento de segundos.

## Layout

Painéis redimensionáveis e recolhíveis (`Ctrl+1` esquerdo, `Ctrl+2` direito; o tamanho é lembrado). **Rail** à esquerda: **Mídia · Texto · Áudio · Transições · IA** (as **Sequências** ficam na aba "Sequências" de Mídia; as **legendas** ficam em Texto). Centro: **Preview** e **Timeline livre**. Direita: **Inspetor** (sem seleção mostra só o nome e o formato da sequência; com um clipe, o essencial — o resto está em "Avançado"). Topo: Desfazer/Refazer, **Exportar**, **Histórico**, **Configurações**, estado de salvamento e tarefas em segundo plano.

## Timeline livre

Não existem tracks fixas: o projeto novo começa vazio. Arraste um vídeo, imagem, texto ou áudio para a timeline — se soltar no espaço vazio (embaixo da última faixa, ou no meio de uma timeline vazia) o CapIA cria a faixa do tipo certo. Quantas faixas quiser, de vídeo e de áudio; as de cima cobrem as de baixo. Dê **dois cliques no nome** da faixa para renomear; **botão direito** no cabeçalho: renomear, mover para cima/baixo, excluir (faixa vazia). Também por faixa: travar, ocultar (vídeo), mudo e solo (áudio). Arrastar um clipe para outra faixa compatível o move; arrastar para o espaço vazio cria uma faixa nova.

## Projeto e sequences

- **Sequences** em árvore com pastas; abas para as abertas; criar, duplicar, renomear, excluir (dá para desfazer).
- Presets de formato: 9:16, 1:1, 4:5, 16:9, ou “Igual à sequence ativa”.
- **Aninhada (nested):** uma sequence pode ser usada como clip em outra. Editar um *master compartilhado* mostra “Editando um master compartilhado (N usos)”; **Tornar único** o separa.

## Mídia (biblioteca)

Importação assíncrona com miniaturas, filtros (Tudo/Vídeo/Áudio/Imagem), ordenação e busca. Selos **Offline** e **Modificado**. **Relink…** (um arquivo), **Relink por pasta…** e **Forçar relink…** ([backup e recuperação](06-backup-e-recuperacao.md)). Arraste da biblioteca para a timeline (o destino é pré-visualizado; “Esse clip não pode ir para lá” quando inválido).

## Timeline

Canvas virtualizado (fluida com milhares de clips). Ferramentas **Selecionar** e **Lâmina**.

| Ação | Como |
|---|---|
| Reproduzir/pausar · shuttle | `Espaço` · `J` / `K` / `L` |
| Quadro a quadro · 10 quadros | `←` `→` · `Shift+←` `Shift+→` |
| Início/fim | `Home` / `End` |
| Dividir no playhead | `Ctrl+B` ou `S` |
| Aparar início/fim até o playhead | `Q` / `W` |
| Excluir · excluir com ripple | `Delete` · `Shift+Delete` |
| Copiar · colar · duplicar | `Ctrl+C` · `Ctrl+V` · `Ctrl+D` |
| Agrupar · desagrupar | `Ctrl+G` · `Ctrl+Shift+G` |
| Snapping | `N` |
| Zoom · ajustar | `=`/`+` `-` · `Shift+Z` |
| Selecionar tudo · limpar seleção | `Ctrl+A` · `Esc` |
| Nova sequence · marcador | `Ctrl+N` · `M` |
| Desfazer · refazer | `Ctrl+Z` · `Ctrl+Shift+Z` ou `Ctrl+Y` |
| Exportar | `Ctrl+E` |

Os atalhos são editáveis em **Configurações → Atalhos de teclado** (conflitos são avisados; há “Restaurar todos os padrões”).

**Tracks:** vídeo, áudio e texto; por track: travar, ocultar, silenciar, solo, magnética, altura. Adicionar track de vídeo/áudio/texto. Tracks de áudio por função: Voz, Música, Efeitos sonoros. Clips em track travada não aceitam edição (“A track está travada”).

## Inspetor

- **Sequence:** tamanho do quadro e taxa de quadros.
- **Clip:** nome, ativo, início, duração, início na fonte; **Básico** (posição X/Y, escala, rotação, opacidade, transição), **Animação** (keyframes com interpolação **Linear**, **Manter** ou **Suave**), **Áudio** (volume em dB, fade in/out, **Separar áudio**), **Velocidade** (e reverso).
- **Texto:** conteúdo, tamanho, negrito, alinhamento, cor, fundo, contorno. Propriedades animadas: editar cria um keyframe no playhead.

## Texto, legendas, transições, áudio

- **Texto:** *Título*, *Legenda*, *Terço inferior*.
- **Legendas manuais:** adicionar, dividir, juntar com a próxima, estilo. (Legendas **automáticas** usam transcrição — ver [IA](03-provedores-de-ia.md).)
- **Transições:** *Dissolver*, *Mergulho no fundo*, *Deslizar para dentro*; exigem clip anterior na mesma track e sobra de mídia suficiente (“Não há sobra de mídia suficiente para essa transição”).
- **Áudio:** volume, fades, separar áudio do vídeo.

## Preview

Qualidade **Auto/540p/720p**, uso de **proxies** (modo de desempenho; o proxy nunca é a fonte do export), áreas seguras, tela cheia, métricas (fps, latência, quadros descartados). O áudio de monitoração roda a 1× (mudo em outras velocidades). Se o preview estiver indisponível, a edição continua.

## Histórico

**Histórico** lista cada transação com o ator (você, `run:<id>`…), permite ir a qualquer ponto (desfazer/refazer em lote) e sobrevive a fechar/reabrir o projeto. Se outro cliente (a API local, por exemplo) alterar o documento, a interface ressincroniza sozinha.

## Idiomas

pt-BR e en (**Configurações → Idioma**).
