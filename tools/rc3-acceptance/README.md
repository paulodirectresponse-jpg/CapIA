# RC3 — aceitação externa com o OpenAI real

Gate **externo**: depende da SUA chave. Nada aqui entra no CI com segredo; o CI só prova o caminho
`pending_external` (sem chave ⇒ nunca "sucesso").

## Como rodar (Windows)

1. No app, conecte o OpenAI uma vez (IA → Conectar). A chave fica no **Credential Manager**.
2. Baixe o artefato `capia-rc3-live-harness` do run do *Installer* (contém `capia-devserver.exe` e
   `openai-real.ps1`).
3. PowerShell:

```powershell
.\openai-real.ps1 -DevServer .\capia-devserver.exe -FfmpegDir "C:\Users\<voce>\AppData\Local\CapIA\resources\ffmpeg" -Vision
```

O script **nunca lê, imprime ou grava a chave**: o `capia-devserver --os-vault` a lê dentro do Rust
(só `capia/provider/openai`, endpoint oficial da OpenAI) e a redige de logs/erros. No fim há uma
varredura por padrão `sk-…` em todos os logs e no resumo.

## Passos e resultado

`connect` (importa modelos + probe + Brain Profile automático) → `chat` ("o que você pode fazer?") →
edição simples (`remova os primeiros 2 segundos…`: proposta → aprovação → aplicar → desfazer exato) →
`vision` (opcional) → `stt_whisper_1` (à parte, com áudio sintetizado localmente) → `secret_scan`.

Saída: `rc3-openai-real-out\summary.json`. Código de saída: `0` tudo ok · `1` falha · `3`
`pending_external` (sem chave; o gate continua ABERTO).

Cole o `summary.json` (sem edição) no relatório do teste humano. Servidor falso **não** fecha este gate.
