# 6. Backup e recuperação

## O que é um projeto

Um projeto `meu-anuncio.capia` é **um único arquivo SQLite** com: documento (snapshot + journal de eventos), **histórico completo** (desfazer/refazer sobrevivem a fechar o app), catálogo de mídia (hash, caminho, metadados — **não** a mídia), registros de IA (Runs, memória de projeto, proveniência) e estado de jobs. **O arquivo de mídia nunca entra no `.capia`** e **nenhuma chave de API** jamais é gravada nele.

Ao lado dele podem existir pastas **laterais**, derivadas do nome do arquivo (`meu-anuncio.capia`):

| Pasta/arquivo | Conteúdo | Descartável? |
|---|---|---|
| `meu-anuncio.capia-cache/` | miniaturas, waveforms, índices de quadros, **proxies**, quadros decodificados, staging de downloads | **Sim.** Pode apagar a qualquer momento (app fechado); é recriado sob demanda. O **proxy nunca é a fonte** do export |
| `meu-anuncio.capia-media/ai/` | mídia **adquirida pela IA** (Gateway) e gerada, com hash | **Não** — é durável; o catálogo guarda o caminho. Versões novas nunca sobrescrevem a anterior |
| `meu-anuncio.capia-wal`, `-shm` | arquivos temporários do SQLite (modo WAL) enquanto o projeto está aberto | não copie isoladamente; veja abaixo |
| `meu-anuncio.capia.v<N>.bak` | cópia criada **antes de uma migração** de projeto (N = schema antigo) | guarde até conferir que o projeto migrado está bom |
| seus vídeos/áudios originais | onde **você** os deixou | **Não** — são a fonte de verdade da mídia |

## Fazer backup

1. **Feche o CapIA** (ou o projeto) — assim o WAL é consolidado no `.capia`.
2. Copie: `meu-anuncio.capia` **+** a pasta `meu-anuncio.capia-media/` (se existir) **+** os **arquivos de mídia originais** referenciados. `-cache/` não precisa.
3. Mantenha mais de uma cópia, em outro disco/nuvem; versione por data. O projeto é pequeno (10.000 clips ≈ poucos MiB); a mídia é o que pesa.
4. Para restaurar: copie de volta (mesmos nomes de pasta lateral ao lado do `.capia`) e abra. Se os caminhos da mídia mudaram, o app marca **Offline** e você usa **Relink por pasta…**.

> Copiar o `.capia` com o app aberto pode pegar um estado sem o WAL. Use a cópia do app fechado, ou a que você fez antes de migrar (`.bak`).

## Integridade e crash

- Cada edição é **uma transação** SQLite com `synchronous=FULL`: queda de energia ou kill do processo nunca deixa um estado pela metade; ao abrir é feita uma verificação de integridade.
- Se o arquivo estiver corrompido, o app informa `PROJECT_CORRUPTED` e **não o altera**. Restaure um backup/`.bak`.
- Exportações escrevem em staging, validam com ffprobe e só então publicam por `rename`: um kill nunca deixa MP4 parcial como arquivo final.
- Runs de IA interrompidas voltam **Pausadas** ao reabrir — nunca retomam sozinhas.

## Migrações (só para frente)

Abrir um projeto de versão antiga o **migra automaticamente**, criando antes `meu-anuncio.capia.v<N>.bak` (`VACUUM INTO`, consistente). Regras:

- **Só para frente:** não existe migração para trás. Um projeto de schema **mais novo** que o app é **recusado** (`UNSUPPORTED_SCHEMA_VERSION`) **sem tocar no arquivo** — atualize o CapIA. Não é possível “abrir só leitura”.
- Falha de migração (`MIGRATION_FAILED`) deixa o arquivo na última versão concluída.
- **Depois de abrir um projeto numa versão nova, não o abra numa versão antiga.** Para voltar de versão, restaure o `.bak` (ou o seu backup) e use o app antigo.
- Tabela de versões e política completa: [`docs/MIGRATION_COMPAT.md`](../MIGRATION_COMPAT.md).

## Atualizar o CapIA e voltar atrás (rollback)

- **Faça backup dos projetos antes de atualizar** (sempre; migrações são de mão única).
- **Fase 6 (em integração; verificação externa pendente):** o atualizador valida assinatura do manifesto e do artefato antes de instalar, instala em duas etapas (preparar → verificar → trocar) e mantém a versão anterior recuperável se o download falhar, o artefato estiver corrompido, a assinatura for inválida ou o app não passar na verificação de saúde ao iniciar; não atualiza no meio de uma Run ativa. Não há downgrade acidental.
- Rollback manual: reinstale a versão anterior **e** restaure o `.capia` (ou o `.bak`) de antes da atualização. Suas configurações e referências de chaves no cofre são preservadas pelo atualizador/instalador; **desinstalar não apaga projetos**.

## Exportar para entregar

Entregue o **MP4**/intermediário exportado, não o `.capia`. Para compartilhar o projeto editável, copie o `.capia` + `-media/` + originais (o `.capia` não contém chaves nem mídia).
