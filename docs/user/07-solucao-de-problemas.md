# 7. Solução de problemas

## FFmpeg não encontrado

Sintoma: banner “O FFmpeg não foi encontrado: importação, preview e exportação ficam indisponíveis. A edição continua funcionando.” ou erro `FFMPEG_NOT_FOUND`.

- No instalador do release candidate o FFmpeg deve vir **embutido** — reinstale/“reparar”. (Verificação em máquina limpa: externa, ver [KNOWN_ISSUES](../KNOWN_ISSUES.md).)
- Em build de desenvolvimento, instale um FFmpeg e coloque `ffmpeg`/`ffprobe` no PATH. O CapIA usa **apenas** configuração LGPL; encoders GPL (`libx264`/`libx265`) **não** são usados e o export diz claramente quando não há encoder aprovado.
- Sem FFmpeg você ainda edita projetos já importados; só não importa, não vê o preview nem exporta.

## WebView2

O CapIA usa o **Microsoft Edge WebView2 Runtime**. Se a janela não abrir ou ficar em branco: instale/atualize o runtime (download oficial da Microsoft, “Evergreen”) e reabra. Windows 11 já traz. O preview usa o caminho rápido (SharedBuffer) quando o WebView2 é recente e **cai automaticamente** para um caminho mais lento quando não — sem perder funcionalidade.

## Mídia offline / relink

Selos **Offline** (arquivo sumiu) e **Modificado** (conteúdo mudou). Clips offline ficam marcados.
- **Relink…** troca por **o mesmo conteúdo** (verificado por hash). Se o conteúdo for diferente: “Esse arquivo tem conteúdo diferente”.
- **Relink por pasta…** procura a pasta toda; casa por **tamanho → impressão → SHA-256**, **nunca por nome**.
- **Forçar relink…** troca por conteúdo **diferente** de propósito: clips que deixarem de caber **bloqueiam** a operação — nada é cortado em silêncio.
- Unidades externas desconectadas aparecem como offline até reconectar.

## Erros do editor

“O projeto mudou nesse meio-tempo. Tente de novo.” (`CONFLICT`: outro cliente editou). “Isso sobreporia outro clip da track” (`OVERLAP`). “A track está travada.” “Não há sobra de mídia suficiente para essa transição” → use *Mergulho* ou duração menor. “O projeto está ocupado” (`STORE_BUSY`) → aguarde. “O arquivo de destino já existe” (`EXPORT_EXISTS`) → marque **Sobrescrever** ou escolha outro nome. Em qualquer erro, **Detalhes técnicos** mostra o código.

## Erros de IA

| Mensagem/código | O que fazer |
|---|---|
| “Nenhum modelo de IA configurado ainda” / `NOT_CONFIGURED` | configure um provider ([03](03-provedores-de-ia.md)) |
| `AI_OFF` / “A IA está desligada” | ligue **Recursos de IA ativados**; o editor funciona sem IA |
| Provider “Sem chave” / 401 | cole a chave de novo; ela é só-escrita e nunca reaparece |
| Modelo sem uma capacidade (ferramentas, visão…) | **Testar capacidades**; escolha outro Brain ou um fallback configurado |
| `TOO_MANY_RUNS` | espere uma Run terminar |
| Run em **Esperando você** | abra a Run e decida ([05](05-aprovacoes-e-orcamentos.md)) |
| Run **Pausada** após reabrir | normal — **Retomar** |
| Custo/limite atingido | estenda o orçamento ou pare |

## Diagnóstico para suporte

- **Hoje:** **Configurações → Copiar diagnóstico** copia só versões e tempos — sem nomes de arquivo, caminhos ou segredos. **IA → Configurar IA… → Diagnóstico redigido** mostra a configuração de IA sem chaves.
- **Fase 6 (em integração):** um **pacote de diagnóstico** de um clique, para você **ver o conteúdo antes de compartilhar**: versão do app, sistema, logs redigidos, id do build e erros estruturados recentes. **Nunca** inclui chaves nem mídia. Os logs ficam em pasta documentada com rotação limitada.
- O relatório de **falhas (crash)** é **opt-in e vem DESLIGADO**; ver [privacidade](08-privacidade-e-seguranca.md).
- Ao relatar um problema informe: versão do CapIA, Windows, o que você fez, o que esperava, o que aconteceu e (se tiver) o código/`request_id`.

## Atalhos ou preferências estranhos

“As preferências estavam corrompidas e voltaram ao padrão” — acontece e é seguro; **Configurações → Atalhos → Restaurar todos os padrões**.

## Se nada funcionar

Feche o app, faça **backup** do `.capia` ([06](06-backup-e-recuperacao.md)) e, se quiser, apague `meu-projeto.capia-cache/` (descartável). Reabra.
