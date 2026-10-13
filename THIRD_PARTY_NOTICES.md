# THIRD PARTY NOTICES

Triagem técnica de licenças, não é parecer jurídico. Política e registro completo: `docs/PROVENANCE.md`.

## OpenCut classic (`apps/editor/`)

- Origem: <https://github.com/OpenCut-app/opencut-classic> — commit `cf5e79e919144200294fb9fed22a222592a0aeea` (2026-05-17), repositório arquivado pelo autor.
- Importado com `git subtree add` (sem squash); tag local `upstream-opencut-cf5e79e9`; remote `upstream-opencut`.
- Licença: **MIT** — Copyright 2025-2026 OpenCut. Texto completo preservado em `apps/editor/LICENSE`.
- Modificações CapIA: commits posteriores ao import em `apps/editor/` (ver `git log -- apps/editor`) e `docs/adoption/PHASE_2A.md`.

### Obrigações de dependências distribuídas com o editor

| Dependência | Licença | Obrigação restante |
|---|---|---|
| `mediabunny` | MPL-2.0 | manter arquivos modificados sob MPL e disponibilizar o fonte deles; não modificamos |
| `soundtouchjs` | LGPL-2.1 | permitir substituição/relink da biblioteca (módulo ES separado); avaliar troca por alternativa permissiva (Fase 2B) |
| `@img/sharp-libvips-*` | LGPL-3.0-or-later | **removida do build local-first** (servidor Next de otimização de imagem) |
| `botid` | sem licença declarada | **removida** |
| modelos Whisper (`@huggingface/transformers`, pesos baixados em runtime) | por modelo | desligado por padrão; licença por modelo a registrar antes de habilitar |
| fontes (Google Fonts via CDN) | por família (OFL/Apache esperadas) | empacotar famílias auditadas ou remover CDN (Fase 2B) |

Dependências permissivas (MIT/Apache-2.0/ISC/BSD/Zlib/Unicode/BlueOak/0BSD): atribuição conforme os respectivos pacotes.
