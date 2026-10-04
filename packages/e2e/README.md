# @capia/e2e

E2E (Playwright) do editor contra o **engine e o FFmpeg reais**.

```
pnpm build && cargo build --release -p capia-devserver     # pré-requisitos
pnpm --filter @capia/e2e exec playwright install chromium   # uma vez (ou CAPIA_CHROMIUM=/caminho/chrome)
pnpm --filter @capia/e2e e2e                                 # devserver + front compilado
CAPIA_E2E_TARGET=tauri CAPIA_DESKTOP_EXE=target/release/capia-desktop.exe pnpm --filter @capia/e2e e2e   # app real (Windows)
CAPIA_PERF_STRICT=1 pnpm --filter @capia/e2e e2e --workers=1 --grep "targets with 5k clips"             # benchmark estrito
```
Suites: `flows` (17 fluxos), `editing`, `crash`, `visual` (capturas em `target/e2e-visual/`), `perf` (`target/perf/phase3-ui-perf.json`). Variáveis: `CAPIA_REQUIRE_H264=1`, `CAPIA_REQUIRE_SHARED_BUFFER=1` (CI Windows), `CAPIA_DEVSERVER_PROFILE=release|debug`.
