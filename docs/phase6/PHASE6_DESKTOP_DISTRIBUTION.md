# CapIA — FASE 6 — DESKTOP, INSTALLER, UPDATE E DISTRIBUIÇÃO

# 1. Product bundle
Bundle desktop app + required runtime/sidecars.

# 2. FFmpeg
Use only approved LGPL configuration from project policy.
Verify shipped binaries/license metadata.

# 3. WebView2
Support installed runtime and documented bootstrap behavior.

# 4. Installer
Generate Windows installer using supported Tauri tooling.
Support install, repair where applicable, uninstall.

# 5. Install locations
Use per-user/system policy deliberately.
No write into arbitrary project dirs.

# 6. First run
Create app config dirs safely.
Do not require AI credentials.

# 7. Upgrade
Preserve user config, projects and secret-store refs.

# 8. Uninstall
Never delete user project files by default.

# 9. Signing pipeline
CI/release workflow accepts signing secret/cert securely.
No cert material in repo/logs.

# 10. Signature verification
Automated verifier checks installer/executable signatures when signing is enabled.

# 11. Unsigned dev artifacts
Clearly labeled dev/test only.

# 12. Auto-update architecture
Signed manifest + artifact.
Client verifies signature before install.

# 13. Update channels
Stable/beta optional.
No accidental downgrade unless rollback procedure.

# 14. Atomic update
Stage, verify, switch, recover.

# 15. Rollback
If startup health fails after update, recovery path documented/tested where supported.

# 16. Update during active Run
Policy: defer or checkpoint; never corrupt active project.

# 17. Offline mode
App functions manually with no network.

# 18. Crash reporting
Off by default or explicit opt-in according to product policy.

# 19. Redaction
No secrets, raw media, prompt contents, project contents unless explicit consent.

# 20. Diagnostic bundle
User-triggered; preview contents before sharing if feasible.

# 21. Clean machine matrix
Windows 10 22H2
Windows 11
Fresh user profile
No developer tools
No Rust/Node requirement.

# 22. Automated VM checks
Install → launch → create project → import → edit → export → uninstall.

# 23. Upgrade matrix
Previous RC → current RC.
Project migration validated.

# 24. Portable project behavior
Opening projects from common paths, spaces, Unicode, external drives.

# 25. File associations
Only if desired; safe handling and no destructive behavior.

# 26. Logging locations
Documented, bounded rotation.

# 27. Application version
Single source of truth propagated to server/API/about/diagnostics.

# 28. Release artifacts
installer, hashes, SBOM/license report, changelog.

# 29. Signing external gate
If cert unavailable, mark external. Engineering pipeline must still be tested with test signing or signature abstraction.

# 30. Definition of Done
Installer/update/crash reporting pipeline implemented and CI validated; external cert/physical clean-machine gates explicitly separated.
