# CapIA — FASE 6 — API REST, MCP, WEBHOOKS E AUTOMAÇÃO EXTERNA

# 1. REST conventions
JSON UTF-8; versioned `/v1`; request-id; structured error:
`{code,message,details?,request_id}`.

# 2. Project endpoints
POST /v1/projects
GET /v1/projects
GET /v1/projects/{id}
POST /v1/projects/{id}/open
POST /v1/projects/{id}/close
GET /v1/projects/{id}/summary

# 3. Asset endpoints
POST /v1/projects/{id}/assets
GET /v1/projects/{id}/assets
GET /v1/projects/{id}/assets/{asset_id}
POST relink/import through safe asset APIs only.

# 4. Sequence/timeline endpoints
GET sequences
GET sequence
GET timeline ranges/digests
POST preview-plan
POST apply-plan
Never raw DB writes.

# 5. AI Run endpoints
POST /runs
GET /runs/{id}
POST /runs/{id}/pause
POST /runs/{id}/resume
POST /runs/{id}/cancel
POST /runs/{id}/approvals
GET plan/review/cost/events.

# 6. Export endpoints
POST exports
GET export status
GET/list deliverables
No path traversal; output root policy.

# 7. Async semantics
Long requests return 202 + id.
Provide status polling and event subscription.

# 8. Event stream
Optional SSE for local clients:
- stage;
- progress;
- approval;
- cost;
- completion.

# 9. Idempotency
Mutating requests accept `Idempotency-Key`.
Persist outcome where needed.
Replay returns same semantic result.

# 10. Optimistic concurrency
Write requests carry expected revision / plan token.
409 on drift.

# 11. Auth
Bearer tokens.
Store only hashes.
Token secret shown once.

# 12. Token management
Create/list/revoke/rotate.
Admin scope required.

# 13. Scope matrix
Every route declares required scope.
Architecture test ensures no route missing authorization metadata.

# 14. Loopback default
Server binds 127.0.0.1 only.
Remote bind opt-in.

# 15. TLS
If remote binding enabled:
document reverse proxy or native TLS policy.
Never send tokens over cleartext non-loopback by default.

# 16. CORS
Disabled/deny by default.
Explicit origins only.

# 17. Upload security
Streaming, quota, timeout, file sniffing, hash, staging.
Reject archive bombs/unsupported where applicable.

# 18. API pagination
Assets, runs, history, logs/events paginated.

# 19. MCP architecture
MCP is an adapter over app service/Engine API, not a second backend.

# 20. MCP tools
Expose typed tools matching supported actions.
All tools have JSON schema, scopes and side-effect metadata.

# 21. MCP resources
Project summaries, Run status, plan/review, sequences, assets.

# 22. MCP auth
Local client token/scopes.
No implicit admin trust.

# 23. MCP parity
For each MCP mutating tool, assert corresponding REST/Engine effect parity.

# 24. Webhook registration
Create/list/update/delete endpoints.
Secret stored write-only.

# 25. Webhook payload
event_id,event_type,occurred_at,project_id,run/export ids,data,version.

# 26. Signature
`X-CapIA-Timestamp`
`X-CapIA-Signature`
HMAC over timestamp + body.

# 27. Replay defense
Receiver examples + validation docs.
Server prevents duplicate event-id retries being treated as new by local dispatcher.

# 28. Delivery log
Persist attempt, status code, latency, next retry, terminal state.

# 29. Webhook test endpoint
Provide local/mock harness, not public echo service.

# 30. Canonical external flow test
REST:
- create project
- upload raw
- add reference
- submit briefing/copy
- start Run
- approve
- finish
- export
- webhook received

MCP:
repeat equivalent task.

# 31. Result parity
Compare:
- project revision
- sequences
- clip graph
- variants
- Run records
- export probe
not prose responses.

# 32. Security tests
- missing token 401
- scope 403
- token revocation
- route privilege escalation
- idempotency replay
- stale revision
- malformed JSON
- path traversal
- upload size
- SSRF
- CORS
- webhook forgery/replay.

# 33. Load tests
Concurrent reads/writes/runs within safe limits.
Backpressure instead of uncontrolled task spawn.

# 34. Shutdown
Graceful:
stop accepting new writes, finish/pause safe operations, persist state, close server.

# 35. API docs
OpenAPI generated/validated.
Examples in curl/PowerShell/JS/Python.

# 36. MCP docs
Tool catalog, scopes, examples, failure modes.

# 37. Definition of Done
REST/MCP/webhook implemented, parity-tested, auth/scopes enforced, canonical flow green on Windows/Linux CI.
