# installer

Pacote de instalação do release candidate: o que a engenharia verifica sozinha e o que depende de **certificado de assinatura real**.

`node tools/phase6-acceptance/installer/run.mjs` — passos em [`steps.json`](steps.json):

| Passo | Tipo | Observação |
|---|---|---|
| `version-single-source` | comando (`check-release.mjs --versions`) | Cargo workspace, `package.json` e `tauri.conf.json` com a MESMA versão; no release, `--expect 0.6.0-rc.1` |
| `release-docs-complete` | comando (`check-release.mjs --docs`) | CHANGELOG com a seção da versão, RELEASE/KNOWN_ISSUES/MIGRATION_COMPAT presentes e coerentes |
| `desktop-build-unsigned` | `pnpm desktop:build`; `CAPIA_P6_HEAVY=1` | artefato de **desenvolvimento** (sem bundle/assinatura) |
| `installer-artifact-inspection` | `inspect-installer.mjs`; exige `CAPIA_INSTALLER` | tamanho, SHA-256 e formato (PE/MSI). **Não** verifica a assinatura digital e não executa o arquivo |
| `dependency-license-policy` | `cargo deny check licenses bans sources` | `not_available` sem `cargo-deny` |
| `signed-installer-and-sbom-evidence` | **externo** (`validate.mjs`) | certificado real, Authenticode válido com carimbo de tempo, SHA256SUMS, SBOM, relatório de licenças, aviso LGPL do FFmpeg |

## Evidência de assinatura

Preencha `target/phase6-acceptance/evidence/installer-results.json` (formato de [`template.json`](template.json)). O validador:

- exige instalador (`nsis|msi`), SHA-256 e tamanho de cada artefato, `SHA256SUMS` conferido, SBOM (`cyclonedx|spdx`), relatório de licenças, build LGPL do FFmpeg verificado (ADR-032) e avisos de terceiros;
- se `signed: true`: exige `authenticode_status: "Valid"`, `signer_subject`, `thumbprint` (SHA-1, 40 hex), `timestamped: true` e o comando de verificação usado (`signtool verify /pa /v` ou `Get-AuthenticodeSignature`);
- artefato **não assinado** (build de teste) ⇒ `partial`: bem formado, mas o gate “instalador assinado” **continua externo**. Sem certificado **não existe `passed`**.

Instalador/atualizador sem assinatura são artefatos de desenvolvimento e devem ser rotulados como tal. Nenhum material de certificado entra no repositório nem em logs (ver `docs/RELEASE.md`).
