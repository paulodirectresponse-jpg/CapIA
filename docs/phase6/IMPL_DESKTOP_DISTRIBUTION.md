# Fase 6 — Track C: desktop, instalador, atualização, crash report e artefatos de release

Estado: **engenharia implementada; partes Windows verificadas só por revisão estática + CI Windows (`installer.yml`)**.
Gates externos na seção 9. Decisões em `docs/phase6/adr-drafts/track-c.md` (rascunhos; integrador promove a `DECISIONS.md`).

## 1. O que existe

| Peça | Onde | Verificado em Linux |
|---|---|---|
| Versão única `0.6.0-rc.1` | `Cargo.toml` `[workspace.package]` → tauri.conf.json, todos os `package.json`, fixture `engine_info.json`, deps de caminho | `pnpm check:version` (+ teste) |
| Build info / diagnóstico / crash opt-in / logs | `crates/capia-support` | testes unitários (18), canário de segredo |
| Atualização assinada | `crates/capia-updater` | 14 unitários + 10 de injeção de falhas + 3 cross-language (Node↔Rust) |
| `--version` e `--self-test` | `apps/desktop/src-tauri/src/{cli,selftest}.rs`, `main.rs` | testes + execução real do self-test com FFmpeg do sistema |
| IPC `support.*`/`update.*` | `apps/desktop/src-tauri/src/support_api.rs` (roteado em `call_json_ai`) | testes |
| UI | `packages/editor-ui` (`PrivacySettings`, `Onboarding`, `store/supportController.ts`), bindings `packages/engine-bindings/src/support.ts` | vitest + regras em `architecture.test.ts` |
| Empacotamento NSIS | `tools/release/stage-bundle.mjs` gera `tauri.installer.generated.json`; `installer-hooks.nsh` | só a geração da config (teste) |
| FFmpeg LGPL | `tools/release/verify-ffmpeg-license.mjs` | testes com binários falsos GPL/LGPL |
| Artefatos | `hashes.mjs`, `sbom.mjs` (CycloneDX + relatório + `THIRD_PARTY_NOTICES`), `make-update-manifest.mjs` | testes |
| Assinatura | `sign.ps1`, `verify-signature.ps1` | **não executável em Linux**; revisão estática (`static-review.test.mjs`) |
| Smoke do instalador | `installer-smoke.ps1` | **não executável em Linux** |
| Workflows | `.github/workflows/installer.yml`, `release.yml` | YAML válido; execução só no GitHub |

`tauri.conf.json` mantém `bundle.active: false` (o CI normal e `pnpm desktop:build` seguem sem instalador). O instalador usa
uma configuração **gerada** e mesclada por `--config`, para não acoplar o build comum a sidecars/FFmpeg.

## 2. Como construir o instalador (Windows)

```
pnpm install --frozen-lockfile
pnpm check:version
node tools/release/sbom.mjs --out-dir dist-release --ffmpeg-dir <ffmpeg-aprovado>
cargo build --release -p capia-server                         # Track A; ver "capia-server" abaixo
pnpm --filter @capia/desktop build
node tools/release/stage-bundle.mjs --ffmpeg-dir <ffmpeg-aprovado> --server-bin target/release/capia-server.exe \
     --webview embed --notices dist-release/THIRD_PARTY_NOTICES.txt
pnpm --filter @capia/desktop tauri build --bundles nsis --config src-tauri/tauri.installer.generated.json
```

* **Sem `e2e-testkit`** (nunca ligar a feature no instalador; `check-architecture` continua guardando isso).
* **Instalação por usuário** (`currentUser`): sem UAC, `%LOCALAPPDATA%\Programs\CapIA`, atualização sem elevação.
* **FFmpeg**: copiado para `<instalação>\ffmpeg\` — exatamente onde `MediaToolchain::locate` procura (`<exe>/ffmpeg`).
  `stage-bundle` **recusa** FFmpeg não aprovado (flags `--enable-gpl`, `--enable-nonfree`, x264/x265, texto de licença GPL,
  `ffmpeg-build.json` ausente em release). `--allow-dev-ffmpeg` existe só para o CI (o FFmpeg do runner é GPL): grava
  `DEV-TEST-ONLY-UNAPPROVED.txt` ao lado e o artefato é rotulado dev/test. `release.yml` não tem esse atalho.
* **`capia-server`** (sidecar, Track A): `--server-bin <exe>` o empacota como `binaries/capia-server-<triple>.exe` (`externalBin`).
  Se o binário ainda não existe: `--allow-missing-server` grava `SERVER-PLACEHOLDER.txt` e o instalador **não** o inclui
  (decisão explícita; sem a opção o staging falha). Remover o placeholder quando o servidor estiver estável.
* **WebView2** (`--webview`): `embed` (padrão de release: bootstrapper de ~2 MB embutido, instala em silêncio se faltar; precisa de rede
  só nesse caso), `download`, `offline` (runtime completo, ~130 MB, para máquinas sem rede), `skip` (só CI). Windows 11 e
  Windows 10 atualizados já têm o runtime.
* **Atalhos/desinstalador**: gerados pelo NSIS do Tauri (menu Iniciar "CapIA"). `installer-hooks.nsh`: o desinstalador **não
  apaga projetos** — projetos nunca ficam no diretório de instalação; se algum `.capia` for encontrado ali, é **copiado** para
  `Documentos\CapIA-recovered-projects` antes da remoção. Dados do app (`%APPDATA%\app.capia.desktop`: preferências, logs,
  banco do app) ficam, a menos que o usuário peça remoção na caixa do desinstalador.
* **Primeira execução**: `setup()` cria `%APPDATA%\app.capia.desktop\{logs,support}` e o `capia-app.db`; nenhuma credencial de IA
  é exigida e a IA começa desligada; falha do serviço de suporte nunca impede o editor.

## 3. `--version` e `--self-test`

`capia-desktop --version [--json] [--out f]` e `capia-desktop --self-test [--require-media] [--out f]`. Headless: sem janela, sem
rede, sem IA. O self-test cria projeto em pasta temporária, aplica edição pelo Command Engine (execute/undo/redo), importa um
WAV gerado (passa pelo ffprobe empacotado), exporta (intermediário), confere o suporte (crash report **desligado por padrão**,
diagnóstico sem canário de segredo) e devolve JSON; código de saída 0/1. `--out` existe porque o executável de release usa o
subsistema `windows` (sem console). `--require-media` falha se o FFmpeg empacotado não estiver ao lado do executável.

## 4. Versão única

Fonte: `[workspace.package] version`. `pnpm check:version` (`tools/release/check-version.mjs`) falha se divergirem:
`tauri.conf.json`, qualquer `package.json`, fixtures `engine_info.json`, `version = "…"` das dependências de caminho,
crates com versão fixa. Consumidores em runtime: `capia_project::engine_info()`, `capia_support::BuildInfo` (sobre/diagnóstico),
`--version`, `support.status`. Mudar a versão = editar o `Cargo.toml` + os JSON (o verificador lista o que falta).

## 5. Atualização assinada (`capia-updater`)

* **Manifesto** (JSON): `schema, product, version, channel (stable|beta), artifact{name,url(https),sha256,size}, min_version?, notes,
  rollback, signature{alg: ed25519, key_id, value(hex)}`. Assinado: Ed25519 sobre o **JSON canônico** (chaves ordenadas, sem
  espaços) sem `signature`; a verificação usa o JSON **recebido** e rejeita campos desconhecidos.
* **Chaves**: várias chaves confiáveis (rotação). A pública de produção entra na **build** (`CAPIA_UPDATE_PUBKEYS="id:hex,id:hex"`
  em tempo de compilação); sem ela o app responde `not_configured` e **nenhum** update é aceito. A privada só em segredo de CI
  (`CAPIA_UPDATE_SIGNING_SEED_HEX`, `CAPIA_UPDATE_KEY_ID`).
* **Política**: semver com pré-release; canal exato; canal `stable` nunca recebe pré-release; **sem downgrade silencioso**;
  downgrade só com manifesto `rollback: true` **e** pedido explícito do usuário; `min_version` exige intermediário; versão revertida
  por falha de saúde não volta sozinha.
* **Máquina de estados persistida** (`update-state.json`, escrita atômica com fsync+rename):
  `Idle → Downloaded → Verified → Staged → Switched → Confirmed`. Assinatura verificada **antes** de baixar; hash/tamanho antes de
  staging e de novo antes de trocar. `recover()` roda a cada abertura e reconcilia estado × `Switcher::installed_version()` ×
  marcador de saúde (`health.json`): sobrevive a morte antes/depois de qualquer efeito (testado em **todos** os pontos de um ciclo
  completo, nos cenários saudável, falha de saúde explícita e laço de falhas).
* **Rollback automático**: `report_unhealthy` (health check do app falhou) ou mais de `max_boot_attempts` (2) inicializações sem
  confirmar ⇒ `Switcher::restore_previous` (reexecuta o instalador anterior retido) e a versão ruim entra em `rejected_versions`.
  *Limitação honesta*: um binário que nem chega a `main()` não consegue disparar o rollback sozinho; o procedimento manual é
  executar o instalador retido em `%APPDATA%\app.capia.desktop\update\previous\`.
* **Política do host** (`HostPolicy`): `Defer` (Run de IA ativa / escrita crítica) mantém o update em `Staged`; `CheckpointRequired`
  chama `checkpoint()` e reavalia. O updater só toca `state_dir`: nunca projetos.
* **Troca real**: `Switcher` (instalador NSIS silencioso `/S`) é o único ponto específico do Windows; testado com falso.
  A integração real (processo, relançamento, retenção do instalador anterior) é o passo documentado a validar no CI Windows /
  máquina limpa. O download (`Downloader`) também é do host; este crate não tem código de rede. No app, `update.check` lê o
  manifesto de `CAPIA_UPDATE_MANIFEST_FILE` (endpoint de produção = pendência externa).

### Procedimento de rollback explícito
1. Gerar manifesto da versão alvo (menor) com `--rollback` e assinar. 2. O usuário pede "instalar versão anterior" (a UI/CLI passa
`allow_rollback = true`). 3. O fluxo é o mesmo (verifica, staging, troca, saúde).

## 6. Crash report e diagnóstico (`capia-support`)

* **Desligado por padrão**; opt-in explícito em "Privacidade e atualizações", persistido no `AppDb` (`ns = support`). O usuário desliga a qualquer momento.
* O gancho de pânico **sempre** grava um registro **local redigido** (versão, SO, arquivo:linha de 3 componentes, mensagem redigida, sem
  backtrace) em `support/crash/`, no máximo 20. O envio só ocorre na **próxima abertura** (`flush_pending`) e **só** com opt-in **e**
  `CrashSink` configurado. O crate não tem rede: o endpoint é externo (`UnconfiguredSink` ⇒ nada sai; a UI diz que os relatórios
  ficam no computador).
* **Diagnóstico**: acionado pelo usuário; `preview()` lista **exatamente** o que entra (arquivo, tamanho, descrição) e o que nunca
  entra; ZIP (`support/diagnostics/`, últimos 5). Conteúdo: `app.json`, `system.json` (sem host/usuário), `settings.json`, cauda dos
  logs, últimos 50 erros estruturados, registros de crash locais, `manifest.json`. Todo texto passa por `capia_secrets::redact_global`
  **na saída** (mesmo segredo escrito cru em um log) e por remoção do diretório pessoal. Testes com canário registrado cobrem bundle,
  log, erro e crash.
* **Logs**: `%APPDATA%\app.capia.desktop\logs\` (`capia.log`, `errors.log`), rotação limitada: 1 MiB × 5 arquivos por log (teto 5 MiB),
  linhas ≤ 8 KiB, redigidas antes de tocar o disco.

## 7. Artefatos de release e assinatura

`tools/release`: `hashes.mjs` (SHA256SUMS + json, `--verify`), `sbom.mjs` (CycloneDX 1.5 de `cargo metadata` + `pnpm-lock.yaml`,
`license-report.json`, `THIRD_PARTY_NOTICES.txt` com a seção FFmpeg), `make-update-manifest.mjs`, `sign.ps1`, `verify-signature.ps1`.
Estados de assinatura: `valid` (release), `test_signed` (certificado autoassinado descartável, prova do pipeline, **não** confiável),
`external_pending` (sem certificado: artefato `*-UNSIGNED-DEV-TEST-ONLY.exe`). Nunca se declara "assinado" sem assinatura.
Certificado: `CAPIA_SIGN_PFX_BASE64` / `CAPIA_SIGN_PFX_PASSWORD` só como segredos de CI (o PFX temporário é apagado; nada vai a log).

## 8. Matriz de máquina limpa (harness)

`tools/release/installer-smoke.ps1` é o harness; roda no CI (`installer.yml`, `windows-latest`) e deve ser executado nas máquinas
físicas/VM abaixo copiando só o instalador + o script (sem Rust/Node):

| Alvo | Perfil | Como | Status |
|---|---|---|---|
| windows-latest (CI) | runner limpo, sem SDKs necessários ao app | `installer.yml` | automatizado |
| Windows 10 22H2 | perfil novo, sem ferramentas de dev | `pwsh -File installer-smoke.ps1 -Installer <setup.exe>` | **EXTERNO** |
| Windows 11 | perfil novo | idem | **EXTERNO** |
| Atualização RC→RC | instalar RC anterior, depois o atual por cima | rodar o smoke, instalar o novo e `--self-test`; validar migração de um `.capia` antigo | **EXTERNO** (CI só tem o RC atual) |
| Caminhos | projeto com espaços/Unicode/disco externo | o smoke já usa nome com espaços e acentos em Documentos | parcial |

Passos do smoke: instalar em silêncio (por usuário) → layout instalado (ffmpeg, notices) → `--version` → `--self-test` do local
instalado → primeira execução (pastas, sem chaves) → criar `.capia` em Documentos → desinstalar em silêncio → **o projeto sobrevive**
→ reinstalar → self-test. Relatório `smoke-report.json`.

## 9. Gates EXTERNOS (não fabricados)

1. **Certificado de code signing real** (aquisição/segredos `CAPIA_SIGN_PFX_*`) — até lá: `signing: external_pending`, artefatos dev/test.
2. **Chave de update de produção** (`CAPIA_UPDATE_PUBKEYS` na build; semente em segredo) e **endpoint de manifestos/artefatos**.
3. **Endpoint de crash report** (implementação de `CrashSink`) — sem ele nada é enviado.
4. **FFmpeg LGPL aprovado** (build própria ADR-032 com `ffmpeg-build.json`): o CI usa o FFmpeg do runner (GPL) só para teste e rotula dev/test.
5. **Máquinas limpas físicas** Windows 10 22H2 / Windows 11 e upgrade RC→RC (seção 8).
6. Decisão jurídica H.264/AAC (herdada), beta humano.
7. Validação do `Switcher` real (NSIS silencioso + relançamento + retenção do instalador anterior) em Windows — o CI executa instalação/desinstalação,
   mas não o fluxo de update ponta a ponta.

## 10. Não verificado localmente (validar no job Windows)

`tauri build --bundles nsis` (config gerada, `installerHooks`, `externalBin`, `webviewInstallMode`), nome do executável instalado
(`CapIA.exe` vs `capia-desktop.exe`; o smoke aceita ambos), `signtool`/`Get-AuthenticodeSignature`, `New-SelfSignedCertificate`,
o hook NSIS de recuperação, abertura do app (WebView2) na primeira execução, e a compilação do `capia-desktop` para Windows com os
novos módulos (compilou e testou em Linux com `webkit2gtk`).
