# ADR drafts — Track C (desktop, instalador, atualização, suporte)

> Rascunhos. O integrador numera e promove para `docs/DECISIONS.md`. Estado: **Proposed**.

### ADR-C1 — Fonte única da versão do app
**Contexto:** versão `0.0.0` espalhada (Cargo, 7 `package.json`, `tauri.conf.json`, fixture do contrato `engine_info`).
**Decisão:** `[workspace.package] version` do `Cargo.toml` é a única fonte (`0.6.0-rc.1`). `tools/release/check-version.mjs` (`pnpm check:version`, no CI do instalador e no release) falha se `tauri.conf.json`, qualquer `package.json`, a fixture, as dependências de caminho do workspace ou um crate com versão fixa divergirem. Em runtime tudo deriva de `CARGO_PKG_VERSION` (`engine_info`, `BuildInfo`, `--version`, diagnóstico, servidor).
**Alternativas:** gerar os JSON a partir do Cargo (rejeitada: esconde divergência e quebra o editor de texto); versão por pacote (rejeitada: sem sentido para um produto único).
**Consequências:** todo bump toca ~15 arquivos (o verificador lista); pré-release `-rc.N` é válida no NSIS (WiX não aceitaria; não usamos).

### ADR-C2 — Instalador NSIS por usuário
**Decisão:** NSIS via Tauri, `installMode: currentUser` (sem UAC; `%LOCALAPPDATA%\Programs\CapIA`; atualização sem elevação, pré-requisito do updater silencioso). Config do instalador **gerada** (`stage-bundle.mjs`) e mesclada por `--config`; `tauri.conf.json` segue com `bundle.active=false`. WebView2: `embedBootstrapper` por padrão em release (offline opcional). FFmpeg LGPL e (quando houver) `capia-server` como recursos/sidecar sob `<instalação>\ffmpeg\` (onde `MediaToolchain` já procura). Desinstalador **nunca** apaga projetos: projetos nunca vivem no diretório de instalação, e o gancho copia qualquer `.capia` encontrado ali para Documentos antes da remoção. Dados do app só saem se o usuário pedir.
**Alternativas:** instalação por máquina (rejeitada: UAC a cada update, projeto de usuário único); MSI/WiX (rejeitado: pré-release inválida, sem ganho).
**Consequências:** sem política corporativa por máquina nesta fase; para ambientes gerenciados, um instalador `perMachine` pode ser gerado depois com a mesma config.

### ADR-C3 — Atualização assinada com máquina de estados persistida
**Decisão:** manifesto JSON assinado com Ed25519 sobre o JSON canônico sem `signature` (ordem de chaves, sem espaços), verificado sobre o JSON recebido; `Verifier` é um trait, implementação `ed25519-dalek` (puro Rust; BSD-3-Clause). Chaves públicas na build (`CAPIA_UPDATE_PUBKEYS`, múltiplas p/ rotação); sem chaves ⇒ `not_configured`. Política: canal exato, `stable` sem pré-release, sem downgrade silencioso (rollback só com manifesto marcado **e** pedido explícito), `min_version`, versões revertidas não voltam. Estados persistidos `Idle→Downloaded→Verified→Staged→Switched→Confirmed` com escrita atômica; `recover()` reconcilia estado × versão instalada × marcador de saúde; rollback automático por falha explícita de saúde ou >2 inicializações sem confirmar. Troca real atrás de `Switcher` (instalador silencioso), download atrás de `Downloader`, política de Runs atrás de `HostPolicy` (adiar/checkpoint). Hash/tamanho conferidos antes do staging e antes da troca.
**Alternativas:** `minisign`/`signify` (rejeitado: formato extra sem ganho aqui); `ring` (rejeitado: não puro Rust); updater do Tauri (rejeitado: pouco controle da máquina de estados/rollback e da política de Runs).
**Consequências:** 100% testável com falsos e injeção de falhas em todos os pontos; a troca real no Windows é o passo a validar; binário que não chega a `main()` precisa de rollback manual documentado.

### ADR-C4 — Crash report opt-in, local-first
**Decisão:** desligado por padrão; opt-in explícito persistido no `AppDb`; gancho de pânico sempre grava registro **local** redigido (versão, SO, arquivo:linha curto, mensagem redigida; sem backtrace, sem conteúdo de projeto/mídia/prompt); envio apenas na abertura seguinte, com opt-in **e** `CrashSink` configurado; `capia-support` não tem código de rede. O usuário pode desligar e apagar os registros.
**Alternativas:** opt-out (rejeitada: política de privacidade do produto); envio dentro do gancho de pânico (rejeitada: frágil e perigoso em estado de pânico).
**Consequências:** endpoint de upload é pendência externa; a UI informa honestamente que, sem destino, os relatórios ficam locais.

### ADR-C5 — Diagnóstico com preview e redação na saída
**Decisão:** o pacote de suporte é acionado pelo usuário, tem `preview()` com a lista exata de arquivos/tamanhos/descrições e a lista do que nunca entra; todo texto é redigido **na saída** (`redact_global` + remoção do diretório pessoal), mesmo que um segredo tenha sido escrito cru num log; só tipos conhecidos de arquivo entram (nada de `.capia`, mídia ou arbitrários); logs com rotação limitada (1 MiB×5), linhas truncadas.
**Alternativas:** confiar só na redação de escrita (rejeitada: logs de terceiros/crashes); incluir projeto "para ajudar" (rejeitada: dado do usuário).
**Consequências:** testes com canário no bundle/log/erro/crash são requisito de regressão.
