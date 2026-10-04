import { defineConfig } from "@playwright/test";

const chromium = process.env.CAPIA_CHROMIUM;

/**
 * E2E do editor: cada worker sobe o seu `capia-devserver` (engine real + FFmpeg real) numa porta
 * livre e serve o front compilado (`apps/desktop/dist`). Pré-requisitos: `pnpm build` e
 * `cargo build -p capia-devserver` (o CI faz os dois antes).
 */
export default defineConfig({
  testDir: "./tests",
  timeout: 90_000,
  expect: { timeout: 10_000 },
  fullyParallel: true,
  ...(process.env.CI ? { workers: 2 } : {}),
  retries: 0,
  reporter: [["list"]],
  use: {
    viewport: { width: 1600, height: 900 },
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
    launchOptions: chromium ? { executablePath: chromium } : {},
  },
  outputDir: "../../target/e2e-results",
});
