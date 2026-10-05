# 8. Privacidade e segurança

## Princípios

- **Local-first:** seus vídeos, áudios e projetos ficam no seu computador. Editar, ver o preview e exportar **nunca** dependem de rede ou de IA.
- **Mínimo necessário na nuvem:** só sobe o que a tarefa de IA pedida precisa — e só se você configurou um provedor.
- **Segredos fora de tudo:** chaves ficam no cofre do sistema (Gerenciador de Credenciais do Windows), nunca no projeto, em logs, em diagnósticos, em Git ou na interface web embutida.

## O que sai da sua máquina

| Situação | O que é enviado | Para onde |
|---|---|---|
| Nenhum provedor configurado (padrão) | **nada** | — |
| Chat/Run de IA | texto do briefing e trechos necessários; transcrição | o provedor **que você configurou** (e só o fallback que você configurou) |
| Transcrição na nuvem | **áudio comprimido** do trecho | o provedor de transcrição configurado; evite com *Transcrever somente localmente* ou *Nunca enviar áudio para a nuvem* |
| Critic com visão | até 6 **quadros** amostrados, ≤ 384 px | o provedor de visão configurado (só se a privacidade permitir) |
| Documentos do briefing | texto extraído | provedor — evite com *Nunca enviar documentos* |
| Fontes aprovadas do Gateway | requisições a URLs/bibliotecas **que você aprovou** | essas URLs |
| Geração de mídia (opt-in) | o prompt de geração | provedor de geração; sempre com sua aprovação |
| Detecção de cenas, remoção de silêncio, análise de ritmo da referência | **nada** (rodam localmente) | — |
| Atualização do app (Fase 6) | consulta ao manifesto de versão | servidor de atualizações |
| **Relatório de falhas (Fase 6)** | **desligado por padrão (opt-in)**; se ligado: versão, SO, metadados de falha e diagnóstico redigido — **sem** mídia, **sem** conteúdo de projeto, **sem** segredos | servidor de relatórios |
| API local / MCP / webhooks (opcional) | só em `127.0.0.1` por padrão; webhooks para a URL **que você** registrar | seu receptor |

**Nunca** sai: sua mídia original inteira, o arquivo `.capia`, suas chaves.

## Segurança

- O texto de briefing, documentos, transcrições e OCR é tratado como **dado**, não como instrução (defesa contra “prompt injection”). A IA só usa uma **lista fechada** de comandos do editor — **sem** shell, sem acesso livre a arquivos, sem HTTP livre, sem acesso a configurações ou chaves.
- A IA só escreve na timeline depois de um **plano validado**, com **aprovação** onde a política exige e **desfazer** sempre.
- O arquivo de mídia é tratado como **entrada hostil**: o FFmpeg roda como processo separado, sem shell, com tempo limite.
- Buscas de rede (provedores, Gateway, webhooks) recusam destinos de rede privada/metadados e **não seguem redirecionamentos** para outro host.
- **Limite reconhecido:** o cofre do sistema não protege contra um malware executando como você.

## API local

Desligada até você iniciar o servidor; escuta só em `127.0.0.1`; exige token com **scopes mínimos**; segredos de token e de webhook são mostrados **uma vez**. Veja [API local](09-api-local.md).

## Estado de verificação

Verificado por CI: chave canário ausente de logs/projeto/arquivos/IPC; IA desligada ⇒ zero chamadas de IA e zero requisições externas no E2E; SSRF/redirect/limites nos clientes de rede. **Externo/pendente:** pentest independente, verificação do relatório de falhas opt-in em build empacotada e revisão jurídica de licenças (FFmpeg/H.264/AAC) — ver [KNOWN_ISSUES](../KNOWN_ISSUES.md). Licenças de terceiros e o aviso do FFmpeg acompanham o instalador (verificação do pacote: externa).

Relatar uma vulnerabilidade: não abra issue pública com detalhes exploráveis; envie ao mantenedor por canal privado, **sem** chaves ou mídia de terceiros ([`docs/api/security.md`](../api/security.md)).
