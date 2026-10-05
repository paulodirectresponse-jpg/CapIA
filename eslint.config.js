import js from "@eslint/js";
import globals from "globals";
import reactHooks from "eslint-plugin-react-hooks";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "**/node_modules/**",
      "**/dist/**",
      "**/target/**",
      "spikes/**",
      "tools/s1-preview-spike/**",
      "apps/desktop/src-tauri/**",
      "docs/**",
    ],
  },
  js.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  {
    languageOptions: {
      parserOptions: { projectService: true, tsconfigRootDir: import.meta.dirname },
      globals: { ...globals.browser },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "@typescript-eslint/consistent-type-imports": "error",
      "@typescript-eslint/no-explicit-any": "error",
    },
  },
  // Playwright: o parâmetro `use` dos fixtures não é um hook do React; Node + DOM no mesmo arquivo.
  {
    files: ["packages/e2e/**/*.ts"],
    languageOptions: { globals: { ...globals.node, ...globals.browser } },
    rules: { "react-hooks/rules-of-hooks": "off", "no-empty-pattern": "off" },
  },
  // Scripts Node (ferramentas, exemplos de integração) e arquivos de configuração: sem type-check de projeto.
  {
    files: ["tools/**/*.mjs", "examples/**/*.mjs", "**/*.config.{js,ts}"],
    ...tseslint.configs.disableTypeChecked,
    languageOptions: {
      ...tseslint.configs.disableTypeChecked.languageOptions,
      globals: { ...globals.node },
    },
  },
);
