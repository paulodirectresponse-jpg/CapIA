# CapIA — FASE 6 — BETA, DOCUMENTAÇÃO E ACEITAÇÃO FINAL

# 1. User onboarding
Install → first project → import → edit → AI setup → autonomous Run → export.

# 2. Docs set
Getting Started
Manual Editor
AI Providers
Autonomy
Approvals/Budgets
REST API
MCP
Webhooks
Backup/Recovery
Troubleshooting
Privacy/Security
Release Notes

# 3. In-app onboarding
Minimal, dismissible, no blocking tour.

# 4. Sample project
Synthetic/licensed assets only.

# 5. API examples
PowerShell, curl, JS, Python.

# 6. MCP examples
Configure client, auth, start Run, query status.

# 7. Webhook example receiver
Local sample server with signature verification.

# 8. Support diagnostics
Instructions for generating redacted bundle.

# 9. Release checklist
version, changelog, migration, installer, signing, hashes, SBOM, CI, known issues.

# 10. Beta acceptance package
`tools/phase6-acceptance/`

Subfolders:
- external-flow
- installer
- update
- security
- performance
- clean-machine
- beta-feedback

# 11. External-flow acceptance
Run same task UI/REST/MCP.
Webhook received.
Compare project/export result.

# 12. Clean-machine acceptance
Scripted checklist for Windows 10/11.
No developer dependencies.

# 13. Beta feedback
Severity, reproducibility, logs, version, workflow, user rating.

# 14. Release quality categories
Blocker, Critical, Major, Minor, Cosmetic.

# 15. Exit gate
No known blocker/critical open for release candidate.

# 16. Real-user beta
External/human gate; do not fabricate.

# 17. Signing acceptance
External if certificate unavailable.

# 18. Update acceptance
Signed update between two RC builds when signing available.

# 19. Rollback acceptance
Interrupted/corrupt update leaves recoverable prior install.

# 20. Docs verification
Fresh tester can complete canonical workflow following docs.

# 21. Accessibility/smoke
Keyboard basics, readable errors, scaling.

# 22. Localization
pt-BR/en release strings remain intact.

# 23. Telemetry/privacy
Crash reporting opt-in behavior verified.

# 24. Legal/licensing bundle
Third-party notices and FFmpeg/license information included.

# 25. Final project compatibility
Open/migrate representative old projects.

# 26. Final status semantics
If all external gates pass: `PHASE 6 COMPLETE`.
If engineering is done but external release gates remain: `PHASE 6 ENGINEERING COMPLETE — EXTERNAL RELEASE ACCEPTANCE PENDING`.

# 27. Final report
Git/CI, API/MCP/webhooks, installer/update, security, performance, docs, beta, external pending, exact release readiness.
