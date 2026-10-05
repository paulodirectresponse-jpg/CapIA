# clean-machine

Aceitação em **máquina limpa**: Windows 10 22H2 (build 19045) e Windows 11, perfil de usuário **novo**, **sem ferramentas de desenvolvedor** (nada de Rust, Node, pnpm, Git, Python, CMake, Visual Studio, nem `ffmpeg` no PATH — o FFmpeg precisa vir embutido). Fluxo: **instalar → abrir → criar projeto → importar → editar → exportar → desinstalar**.

> **Status: externo.** Esta engenharia não tem máquinas Windows 10/11 limpas. Nenhum resultado foi gerado. O validador responde `pending_external` sem arquivo real.

## Como executar (na máquina de teste)

1. Copie para a máquina **só** o instalador do RC e `run-clean-machine.ps1` (não precisa de Node: é PowerShell 5.1+, que já vem no Windows).
2. `powershell -ExecutionPolicy Bypass -File .\run-clean-machine.ps1 -Installer .\CapIA_0.6.0-rc.1_x64-setup.exe -Performer "Seu nome" -Kind physical`
   - `-DryRun` só mostra o inventário (SO e ferramentas) — use antes para confirmar que a máquina é limpa.
   - Instalação/desinstalação silenciosas opcionais: `-InstallCommand '"{0}" /S'`, `-UninstallCommand '…'`; sem eles o script pede para você fazer manualmente.
3. O script coleta **automaticamente** versão do Windows, ferramentas de dev presentes, SHA-256 e assinatura Authenticode do instalador, tempos, tamanho do export e se o projeto sobreviveu à desinstalação; **pergunta** o que só um humano vê (criar, importar, editar, exportar). Responda com a verdade: `n` e `x` são resultados válidos que o validador registra como falha.
4. Grava `clean-machine-results.json`. Repita em Windows 10 22H2 **e** Windows 11 (acrescente a segunda máquina ao array `machines`, ou rode duas vezes e una os arquivos).

## Validação

`node tools/phase6-acceptance/clean-machine/validate.mjs --file clean-machine-results.json` (ou coloque em `target/phase6-acceptance/evidence/clean-machine-results.json`).

| Resultado | Quando |
|---|---|
| `pending_external` | sem arquivo, ou ainda é o modelo (`"template": true`) |
| `rejected` | malformado; máquina **não limpa** (qualquer ferramenta de dev presente); SO incoerente (Windows 10 precisa ser 22H2/19045; Windows 11 build ≥ 22000); passo falho ou não executado; export vazio; desinstalador apagou o projeto do usuário; atestados ausentes; versão ≠ RC |
| `partial` | tudo bem formado e aprovado, mas só uma das duas versões do Windows |
| `accepted` | Windows 10 22H2 **e** Windows 11, os 7 passos `passed` em cada um, atestados reais |

O resumo informa quantas máquinas são **físicas** e se há instalador **não assinado** (isso não atende o critério “instalador assinado”). `kind: "vm"`/`"ci_image"` são aceitos como evidência, mas a verificação física pode ser exigida pelo gate de release.

Limites honestos: os atestados (`results_are_real`, `no_simulation`, `fresh_user_profile`) são declarações do executor; o script **não os marca sozinho**. O script não consegue provar que o perfil é novo.
