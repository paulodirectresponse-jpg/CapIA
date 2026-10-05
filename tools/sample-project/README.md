# Projeto de exemplo sintético

`node tools/sample-project/make-sample.mjs [pasta] [--no-project] [--dry-run]`

Gera, **sem nenhum conteúdo de terceiros** (tudo vem de fontes `lavfi` do FFmpeg e de um texto escrito para isto):

- `media/raw-talking-head.mp4` (12 s, vertical, com “voz” sintética), `raw-broll-1.mp4`, `raw-broll-2.mp4`, `music.wav`, `logo.png`;
- `media/reference-ad.mp4` — referência com 3 planos de cor e cortes secos (útil para o Reference Analyzer);
- `briefing.txt` — briefing curto de um produto **fictício** (“Foco Diário”) com os campos que o Demand Interpreter extrai;
- `sample.capia` — projeto criado e com todos os assets importados pela CLI `capia` (`create`, `asset import`, `asset list`).

É **opcional**: sem `ffmpeg` no PATH imprime que foi **pulado** e sai com 0; sem a CLI `capia` (e sem `cargo`) gera só a mídia e o briefing. A CLI é encontrada por `CAPIA_CLI_BIN`, `target/release|debug/capia` ou `cargo run -p capia-cli`. `--dry-run` imprime os comandos sem executar nada.

Para experimentar: abra `sample.capia` no CapIA, crie uma sequence 9:16, arraste os clips; com IA configurada, inicie uma AI Run colando o `briefing.txt` e selecionando o bruto e a referência. O script **não** monta a timeline (isso é do editor/da IA). Testes: `tools/sample-project/make-sample.test.mjs` cobre só as partes puras (plano do ffmpeg/CLI, briefing, resolução da CLI, pulo sem ffmpeg).
