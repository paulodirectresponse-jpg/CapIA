# 3. Provedores de IA

**A IA é opcional.** Com a IA desligada (padrão até você configurar algo) o editor inteiro — importar, editar, preview, exportar — funciona normalmente, sem rede. O banner “A IA está desligada. O restante do editor continua funcionando.” é informativo.

## Onde ficam as chaves

- A **chave de API** vai **uma única vez** do campo da interface para o **cofre do sistema operacional** — no Windows, o **Gerenciador de Credenciais**. Depois disso a interface só sabe “Chave salva / Sem chave”; nenhuma tela mostra a chave de novo.
- A chave **nunca** é gravada no arquivo do projeto (`.capia`), em logs, em diagnósticos, em Git, nem enviada para a WebView; ela é vinculada ao **host** do provedor e não é usada por outro.
- Fora do Windows (sem cofre do SO), a interface informa “As chaves ficam só na memória e se perdem ao fechar o app.”
- Limite reconhecido: o cofre protege em repouso e entre usuários, não contra um malware rodando como **você** na mesma sessão.

## Configurar (IA → Configurar IA…)

1. **Providers → Adicionar provider**: escolha um **modelo pronto** — OpenAI, Anthropic, Google Gemini, OpenRouter, Groq, **Ollama (local)**, **LM Studio (local)** ou **servidor whisper.cpp (local, só transcrição)** — ou **Personalizado** (família: OpenAI-compatível, Anthropic, Google; **URL base**). Para provedores locais em `localhost`, marque **Permitir endereço local (loopback)**.
2. Cole a chave (só escrita) e **Salvar provider**. **Remover chave** apaga do cofre.
3. **Importar modelos** ou **Adicionar modelo** (id e janela de contexto) e **Testar capacidades**: o CapIA mede de verdade texto, streaming, ferramentas, saída estruturada, visão, entrada de áudio e transcrição. Cada capacidade mostra a origem: *predefinida*, *declarada* ou *verificada*.
4. **Perfil do Brain:** o **Brain principal** (modelo que planeja/escreve) e o **fallback para texto**. O CapIA **só** usa o fallback que você configurou — nunca troca de modelo por conta própria.
5. **Privacidade** (marque o que quiser proibir):
   - *Transcrever somente localmente*;
   - *Nunca enviar áudio para a nuvem*;
   - *Nunca enviar documentos*.
6. **Custo máximo por tarefa** (em micro-unidades da moeda do provedor) e o painel **Uso** (chamadas, tokens, custo; “desconhecido” quando o provedor não declara preço — desconhecido **não** é tratado como zero).
7. **Diagnóstico redigido**: informações de configuração sem segredos.

## O que sai da sua máquina quando você usa IA

Só o mínimo para a tarefa pedida: texto do briefing e de documentos (a menos que “Nunca enviar documentos”), **áudio comprimido** para transcrição (a menos que local/“Nunca enviar áudio”) e **quadros amostrados** (no máximo 6, ≤ 384 px) quando o Critic usa visão. **Sua mídia original nunca é enviada inteira.** Detalhes em [Privacidade e segurança](08-privacidade-e-seguranca.md).

## Ferramentas que funcionam sem provedor

**IA → Ferramentas de IA**: **Remover silêncios** e **Detectar cenas** rodam **neste computador**, sem provedor. **Transcrever**, **Legendas automáticas** e **Analisar como referência** (a análise de cortes/ritmo é local) usam transcrição conforme sua configuração.

## Verificado × pendente

Verificado por CI: o Brain troca entre OpenAI-compatível, Anthropic e Google só por configuração (servidores HTTP falsos que falam cada protocolo); a chave canário nunca aparece em logs, projeto, IPC nem arquivos. **Pendente/externo:** qualidade, custo e latência com **chaves e modelos reais**, e transcrição real medida em áudio de fala real (`docs/KNOWN_ISSUES.md`).
