# 1. Primeiros passos

Roteiro completo: **instalar → primeiro projeto → importar → editar → configurar IA → Run autônoma → exportar**. Você só precisa de IA a partir do passo 5; os passos 1–4 e 7 funcionam sem rede.

## 1. Instalar

- **Windows 10 22H2 ou Windows 11**, com o **WebView2 Runtime** (já vem no Windows 11 e na maioria dos Windows 10 atualizados; o app avisa se faltar — veja [solução de problemas](07-solucao-de-problemas.md)).
- **FFmpeg** é necessário para importar, ver o preview e exportar. O instalador do release candidate deve trazê-lo **embutido** (build própria, LGPL). Sem ele o CapIA abre, mostra o aviso “O FFmpeg não foi encontrado: importação, preview e exportação ficam indisponíveis” e **a edição continua funcionando**.
- **Estado do instalador (Fase 6):** o pipeline do instalador, da assinatura e do atualizador está em integração; um instalador **assinado** depende de um certificado de assinatura real e a verificação em Windows 10/11 limpos é externa (ver [KNOWN_ISSUES](../KNOWN_ISSUES.md)). Builds de desenvolvimento não são assinadas — o Windows pode mostrar o aviso do SmartScreen; só execute binários cujo hash (SHA-256) confira com o publicado na página do release.
- Desinstalar **nunca apaga seus projetos** (eles ficam onde você os salvou).

## 2. Primeiro projeto

Ao abrir sem projeto aparece **Bem-vindo ao CapIA**:

1. **Novo projeto** → informe o **caminho do arquivo do projeto** (ex.: `C:\Videos\meu-anuncio.capia`) → **Criar**. (**Abrir projeto** abre um `.capia` existente; **Recentes** lista os últimos.)
2. O projeto é um **único arquivo `.capia`**. Salvamento é automático e transacional: o indicador no topo mostra *Salvando…* / *Salvo*.
3. Crie uma **sequence** (painel **Projeto → Nova sequence**) a partir de um preset: **9:16 Vertical**, **1:1 Quadrado**, **4:5 Retrato** ou **16:9 Horizontal**.

## 3. Importar mídia

- Painel **Mídia → Importar mídia** (use **Por caminho…** para informar os arquivos, um por linha). Aceita vídeo, áudio e imagem.
- **A mídia fica onde está**; o projeto guarda só a referência e o **hash do conteúdo** (identidade por conteúdo, não por nome). Se mover o arquivo, use **Relink** (ver [backup e recuperação](06-backup-e-recuperacao.md)).
- A importação roda em segundo plano (contador “N tarefa(s) em segundo plano” no topo); aparecem miniaturas e duração.

## 4. Editar

Arraste o clip da biblioteca para a timeline, dê **play** (`Espaço`), **divida** no playhead (`Ctrl+B` ou `S`), apare (`Q`/`W`), adicione **texto/legendas/transições** nos painéis laterais e ajuste no **Inspetor**. Tudo se desfaz (`Ctrl+Z`). Detalhes em [Manual do editor](02-editor-manual.md).

## 5. Configurar a IA (opcional)

**IA** (rail) → **Configurar IA…**: escolha um provedor (OpenAI, Anthropic, Google Gemini, OpenRouter, Groq, ou local como Ollama/LM Studio), cole a **chave de API** — ela vai **uma única vez** ao cofre seguro do sistema e **nunca** é exibida de novo nem gravada no projeto — e **teste as capacidades** do modelo. Defina o **Brain** (modelo principal). Passo a passo e privacidade: [Provedores de IA](03-provedores-de-ia.md). Sem configurar nada, a IA fica desligada e o resto do editor segue normal.

## 6. Primeira Run autônoma

Pré-requisitos: IA configurada e **material bruto importado**.

1. Painel **IA → Execuções → Nova execução**.
2. Escreva o **Briefing** (produto, público, oferta, chamada para ação) e informe a **duração máxima**; selecione o **material bruto** (assets). Opcional: **Analisar como referência** em um vídeo de referência.
3. Mantenha marcado **“Pedir minha aprovação do plano antes de editar”** (recomendado). “Permitir buscar mídia que falta em fontes aprovadas” e “Permitir gerar mídia com IA (pode custar dinheiro)” vêm desligados/protegidos — ligue só se quiser.
4. **Iniciar execução**. A Run passa por *Entender → Planejar → Validar → Obter mídia → Editar → Revisar → Corrigir → Pronto*. Quando precisar de você ela fica **“Esperando você”** com a pergunta e as consequências de cada opção ([Aprovações](05-aprovacoes-e-orcamentos.md)).
5. O resultado são **sequences comuns e editáveis** (“Resultado (sequences editáveis)” → **Abrir**). Nada é escrito na sua timeline antes de o plano ser validado. **Desfazer esta execução** reverte o que a Run fez ([Autonomia](04-autonomia.md)).

## 7. Exportar

**Exportar** (`Ctrl+E`) → escolha a **sequence**, o **preset** (**MP4 (H.264)** ou **Intermediário**), tamanho, **encoder** (automático = primeiro aprovado) e o **arquivo de destino** → **Iniciar exportação**. O arquivo só aparece no destino final depois de **validado com ffprobe**; cancelar não deixa arquivo parcial. Para vários formatos de uma vez use **Entregáveis → Exportar todos**.

> **Limite honesto:** só encoders **aprovados** são usados. Se a sua máquina não tiver nenhum encoder H.264 aprovado, a interface diz “Nenhum encoder H.264 aprovado está disponível nesta máquina” — não há fallback silencioso. O suporte técnico ao H.264 **não** resolve o licenciamento de patentes; isso é decisão de produto/jurídico ainda pendente.

## Próximos passos

[Manual do editor](02-editor-manual.md) · [Aprovações e orçamentos](05-aprovacoes-e-orcamentos.md) · [Solução de problemas](07-solucao-de-problemas.md)
