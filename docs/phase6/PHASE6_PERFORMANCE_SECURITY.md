# CapIA — FASE 6 — PERFORMANCE, SEGURANÇA E HARDENING

# 1. Large-project fixture
≥30 sequences, ≥5,000 clips, nested, captions, audio, Run history, assets.

# 2. Measure
open, save, query, timeline, preview, REST, MCP, run event handling, export queue.

# 3. Preserve Phase 3 targets
No material regression without documented reason/fix.

# 4. API latency
Local reads/writes measured excluding heavy jobs.

# 5. Concurrency
Multiple clients; one-authority engine semantics; revision conflicts handled.

# 6. Memory/handle soak
Repeated open/close and server requests.

# 7. Server soak
8h synthetic workload in nightly/manual if too costly for main CI.

# 8. Export soak
Queue multiple deliverables, cancellation, disk-full simulation where feasible.

# 9. Security threat model
Local attacker, malicious client, malicious media, compromised webhook endpoint, prompt injection, token theft.

# 10. Token security
Strong random secrets, hashing, rotation, expiration/revocation.

# 11. Scope tests
Property/mutation tests for authorization matrix.

# 12. Path traversal
All file/output paths normalized and constrained.

# 13. SSRF
Gateway/server URL handling reuses SafeFetcher policies.

# 14. Upload abuse
Size/time/concurrency limits, malformed media, decompression bombs where applicable.

# 15. JSON/body limits
Bound request size and nesting.

# 16. Webhook attacks
Forgery, replay, redirect, slow endpoint, DNS changes.

# 17. CORS/host header
Deny unexpected.

# 18. Local network exposure
Remote bind explicit only; warning and auth mandatory.

# 19. Secret leakage
Canary through REST, MCP, webhook, crash report, diagnostics, installer logs.

# 20. API fuzzing
Malformed methods/routes/bodies/headers.

# 21. MCP fuzzing
Unknown tools, invalid schemas, oversized args, prompt injection via tool strings.

# 22. Crash resilience
Kill server during:
upload, apply, Run update, webhook retry, export, update staging.

# 23. Disk failure
Read-only disk, full disk, permission denied.

# 24. Migration failure
Atomic recovery/backups.

# 25. Mutation tests
auth bypass, scope ignored, idempotency off, signature verification off, stale revision accepted, redaction disabled.

# 26. Pentest basic report
Document methodology/findings/fixes/residual risks.

# 27. Dependency security
cargo audit/deny, npm audit policy, SBOM.

# 28. Release secret policy
Signing/webhook/provider secrets only via secure CI secret store.

# 29. Performance regression gate
Benchmark thresholds with stable statistical tolerances.

# 30. Definition of Done
Large-project targets measured; security suite green; no known critical/high unmitigated findings.
