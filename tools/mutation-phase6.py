#!/usr/bin/env python3
"""Mutation testing manual da Fase 6 (segurança do servidor REST). Uma alteração por vez; roda os
testes que devem detectá-la e restaura o arquivo (sempre). DETECTED = algum teste falhou e o build
não quebrou; SURVIVED = nenhum teste notou (um achado: falta teste); BROKE-BUILD = a mutação não
compila (inválida, não conta como detectada).
Uso: `python3 tools/mutation-phase6.py [id ...]` (árvore limpa; exige ~1-2 min por mutação: cada
uma recompila `capia-server` e o binário de teste; rode uma instância por vez).
Saída: `target/mutation-phase6.json` com [id, nome, veredito, segundos]."""
import json
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
S = "crates/capia-server/src"
ENV = "CARGO_INCREMENTAL=0 CARGO_PROFILE_DEV_DEBUG=0 CARGO_PROFILE_TEST_DEBUG=0"
SR = f"{ENV} cargo test -q -p capia-server --test security_rest --"
CN = f"{ENV} cargo test -q -p capia-server --test secret_canary --"
RC = f"{ENV} cargo test -q -p capia-server --test rest_core --"
UNIT = f"{ENV} cargo test -q -p capia-server --lib --"


def m(id, name, file, edits, cmd):
    """`edits`: lista de (old, new) aplicados no mesmo arquivo (cada `old` deve existir 1+ vez)."""
    return dict(id=id, name=name, file=file, edits=edits, cmd=cmd)


M = [
    m(1, "auth: qualquer bearer vira admin (bypass)", f"{S}/auth.rs",
      [('    let deny = || ApiErr::unauthorized("missing, invalid, expired or revoked token");\n',
        '    let deny = || ApiErr::unauthorized("missing, invalid, expired or revoked token");\n'
        '    if !bearer.is_empty() {\n        return Ok(Principal { token_id: "tok_any".into(), name: "any".into(), '
        'scopes: crate::scope::ALL_SCOPES.into_iter().collect() });\n    }\n')],
      f"{SR} authorization_header_tricks_never_authenticate every_auth_failure_looks_the_same"),
    m(2, "auth: token revogado continua valendo", f"{S}/auth.rs",
      [("if row.revoked_ms.is_some() || row.expires_ms.is_some_and(|e| e <= now) {",
        "if row.expires_ms.is_some_and(|e| e <= now) {")],
      f"{SR} revocation_takes_effect_immediately_even_under_concurrency every_auth_failure_looks_the_same"),
    m(3, "auth: expiração ignorada", f"{S}/auth.rs",
      [("if row.revoked_ms.is_some() || row.expires_ms.is_some_and(|e| e <= now) {",
        "if row.revoked_ms.is_some() {")],
      f"{SR} expiry_is_enforced_and_rotation_kills_the_old_secret_at_once every_auth_failure_looks_the_same"),
    m(4, "gate: escopo ignorado", f"{S}/core.rs",
      [("            if !p.has(scope) {", "            if false && !p.has(scope) {")],
      f"{SR} scope_matrix_exactly_the_declared_scope_opens_each_operation authorization_decision_equals_membership"),
    m(5, "gate: validação de schema desligada", f"{S}/core.rs",
      [("            if !errs.is_empty() {", "            if false && !errs.is_empty() {")],
      f"{SR} hostile_json_bodies_never_cause_a_5xx a_token_can_never_mint_or_rotate_beyond_its_own_scopes "
      f"&& {RC} host_cors_and_request_shape_are_locked_down"),
    m(6, "rate limit desligado", f"{S}/ratelimit.rs",
      [("        if b.tokens >= 1.0 {", "        if true {")],
      f"{RC} rate_limits_answer_429_with_retry_after"),
    m(7, "idempotência desligada (chave nunca usada)", f"{S}/core.rs",
      [("        let hash = sha256_hex(format!(\"{}\\n{}\", def.name, params).as_bytes());\n",
        "        let hash = sha256_hex(format!(\"{}\\n{}\", def.name, params).as_bytes());\n        let key: Option<&str> = None;\n")],
      f"{SR} concurrent_requests_with_the_same_key_execute_exactly_once"),
    m(8, "idempotência: replay ignora divergência do pedido", "crates/capia-store/src/serverdb.rs",
      [("if o != op || h != request_hash {", "if false && (o != op || h != request_hash) {")],
      f"{RC} idempotency_keys_replay_reject_mismatch_and_never_store_secrets"),
    m(9, "segredo único guardado na tabela de idempotência", f"{S}/core.rs",
      [("strip_secret(&mut stored);", "let _ = &mut stored;")],
      f"{RC} idempotency_keys_replay_reject_mismatch_and_never_store_secrets"),
    m(10, "revisão obsoleta aceita (expected_revision ignorado)", f"{S}/ops.rs",
      [("            if exp != base {", "            if false && exp != base {")],
      f"{SR} stale_revisions_and_foreign_plans_are_rejected"),
    m(11, "Host não validado (rebinding)", f"{S}/server.rs",
      [("        Some(h) if host_allowed(core, h, port) => {}", "        Some(_) => {}")],
      f"{SR} dns_rebinding_hosts_and_origins_are_refused"),
    m(12, "CORS/Origin não validado", f"{S}/server.rs",
      [("    if origin.is_some() && origin_ok.is_none() {", "    if false && origin.is_some() && origin_ok.is_none() {")],
      f"{SR} dns_rebinding_hosts_and_origins_are_refused"),
    m(13, "sanitização de nome: caracteres perigosos mantidos", f"{S}/uploads.rs",
      [("            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ' ') {", "            if true {")],
      f"{SR} hostile_filenames_never_escape_the_data_dir_or_name_special_files filename_sanitizer_properties_hold_for_random_input"),
    m(14, "sanitização: nomes reservados do Windows/metadados liberados", f"{S}/uploads.rs",
      [("    } else if is_reserved_name(&out) {", "    } else if false && is_reserved_name(&out) {")],
      f"{SR} hostile_filenames_never_escape_the_data_dir_or_name_special_files filename_sanitizer_properties_hold_for_random_input"),
    m(15, "sniffing desligado (tudo vira vídeo)", f"{S}/uploads.rs",
      [("pub fn sniff(head: &[u8], filename: &str) -> Option<&'static str> {",
        "pub fn sniff(head: &[u8], filename: &str) -> Option<&'static str> {\n    if !head.is_empty() {\n        return Some(\"video\");\n    }\n")],
      f"{SR} disguised_and_polyglot_files_are_classified_by_content_only"),
    m(16, "limite de corpo JSON desligado", f"{S}/server.rs",
      [("        if body_len > core.cfg.max_json_bytes as u64 {", "        if false && body_len > core.cfg.max_json_bytes as u64 {")],
      f"{RC} host_cors_and_request_shape_are_locked_down"),
    m(17, "limite de profundidade JSON desligado", f"{S}/server.rs",
      [("        if http::json_depth_exceeds(&body, core.cfg.max_json_depth) {", "        if false && http::json_depth_exceeds(&body, core.cfg.max_json_depth) {")],
      f"{SR} hostile_json_bodies_never_cause_a_5xx"),
    m(18, "token guardado em texto claro", f"{S}/auth.rs",
      [("        secret_hash: sha256_hex(secret.as_bytes()),", "        secret_hash: secret.clone(),"),
       (".token_by_hash(&sha256_hex(bearer.as_bytes()))?", ".token_by_hash(bearer)?")],
      f"{SR} only_the_hash_of_a_token_ever_reaches_the_disk"),
    m(19, "redator desligado em ApiErr::body", f"{S}/error.rs",
      [('"message": capia_secrets::redact_global(&self.message),', '"message": self.message.clone(),')],
      f"{CN} error_bodies_redact_secrets_even_when_the_message_carries_one"),
    m(20, "gate de projeto aberto desligado", f"{S}/core.rs",
      [("            if open.as_deref() != Some(want) {", "            if false && open.as_deref() != Some(want) {")],
      f"{RC} only_the_open_project_accepts_project_routes"),
    m(21, "dedupe de upload vaza entre tokens", f"{S}/uploads.rs",
      [('                && m["token_id"] == token_id.as_str()\n', "")],
      f"{SR} staging_quota_zero_byte_checksum_and_dedupe_isolation"),
    m(22, "tokens.create sem checagem de escalada", f"{S}/ops.rs",
      [('        if !escalated.is_empty() {\n            return Err(ApiErr::forbidden(\n                "SCOPE_ESCALATION",\n                format!(\n                    "a token cannot grant',
        '        if false && !escalated.is_empty() {\n            return Err(ApiErr::forbidden(\n                "SCOPE_ESCALATION",\n                format!(\n                    "a token cannot grant')],
      f"{SR} a_token_can_never_mint_or_rotate_beyond_its_own_scopes"),
    m(23, "tokens.rotate sem checagem de escalada", f"{S}/ops.rs",
      [('        if !escalated.is_empty() {\n            return Err(ApiErr::forbidden(\n                "SCOPE_ESCALATION",\n                format!(\n                    "rotating this token',
        '        if false && !escalated.is_empty() {\n            return Err(ApiErr::forbidden(\n                "SCOPE_ESCALATION",\n                format!(\n                    "rotating this token')],
      f"{SR} a_token_can_never_mint_or_rotate_beyond_its_own_scopes"),
    m(24, "Authorization duplicado aceito", f"{S}/http.rs",
      [(' || count("authorization") > 1 {', " {")],
      f"{SR} authorization_header_tricks_never_authenticate"),
    m(25, "upload truncado vira upload completo", f"{S}/uploads.rs",
      [("        if declared_len.is_some_and(|n| size != n) {", "        if false && declared_len.is_some_and(|n| size != n) {")],
      f"{SR} a_truncated_upload_is_never_stored_as_if_it_were_complete"),
    m(26, "SSRF: IPv4 embutido em IPv6 não normalizado", "crates/capia-ai/src/http.rs",
      [("            IpAddr::V6(v6) => v6.to_ipv4_mapped().map_or(ip, IpAddr::V4),", "            IpAddr::V6(_) => ip,")],
      f"{SR} ssrf_targets_are_refused_at_registration_and_update_in_every_mode"),
    m(27, "SSRF: NAT64/6to4/IPv4-compatível liberados", "crates/capia-ai/src/http.rs",
      [("    if s[..6] == [0; 6] && ip != Ipv6Addr::LOCALHOST {", "    if false && s[..6] == [0; 6] && ip != Ipv6Addr::LOCALHOST {")],
      f"{SR} ssrf_targets_are_refused_at_registration_and_update_in_every_mode"),
    m(28, "chave de idempotência gravada em claro", f"{S}/core.rs",
      [('            .map(|k| format!("ik_{}", &sha256_hex(k.as_bytes())[..40]));', "            .map(str::to_owned);")],
      f"{SR} idempotency_keys_are_scoped_per_token_and_never_cross_over"),
    m(29, "redação de segredos na entrada desligada", f"{S}/core.rs",
      [("        let params = redact_registered(params);", "        let params = params;")],
      f"{CN} the_canary_never_comes_back_through_any_input_path a_server_minted_token_is_a_canary_too"),
    m(30, "teto de uploads simultâneos desligado", f"{S}/uploads.rs",
      [("        if self.uploads_active.fetch_add(1, Ordering::SeqCst) >= self.cfg.max_concurrent_uploads {",
        "        if self.uploads_active.fetch_add(1, Ordering::SeqCst) >= usize::MAX {")],
      f"{SR} a_stalled_upload_times_out_and_frees_the_worker_and_the_slot"),
    m(31, "cota de staging desligada", f"{S}/uploads.rs",
      [("        if used.saturating_add(declared_len.unwrap_or(0)) > self.cfg.upload_quota_bytes {",
        "        if false && used.saturating_add(declared_len.unwrap_or(0)) > self.cfg.upload_quota_bytes {"),
       ("            if used.saturating_add(size) > self.cfg.upload_quota_bytes {",
        "            if false && used.saturating_add(size) > self.cfg.upload_quota_bytes {")],
      f"{SR} staging_quota_zero_byte_checksum_and_dedupe_isolation"),
    m(32, "URL de webhook não canônica aceita", f"{S}/ops.rs",
      [("        if rest.is_none_or(|r| r.starts_with(['/', '\\\\']) || url.contains(char::is_whitespace)) {",
        "        if false && rest.is_none_or(|r| r.starts_with(['/', '\\\\']) || url.contains(char::is_whitespace)) {")],
      f"{SR} ssrf_targets_are_refused_at_registration_and_update_in_every_mode"),
    m(33, "validate(): bind remoto sem as duas confirmações", f"{S}/config.rs",
      [("        if !self.is_loopback() && !(self.allow_remote && self.remote_tls_terminated_by_proxy) {",
        "        if !self.is_loopback() && !(self.allow_remote || self.remote_tls_terminated_by_proxy) {")],
      f"{SR} remote_bind_needs_both_confirmations_and_dangerous_configs_are_refused"),
    m(34, "token na query string passa a valer", f"{S}/server.rs",
      [("fn bearer(req: &Request) -> Option<&str> {\n    let h = req.header(\"authorization\")?;",
        "fn bearer(req: &Request) -> Option<&str> {\n    if let Some((_, t)) = req.query.iter().find(|(k, _)| k == \"token\") {\n        return Some(t.as_str());\n    }\n    let h = req.header(\"authorization\")?;")],
      f"{SR} authorization_header_tricks_never_authenticate"),
    m(35, "idempotência: pendente órfão não vira indeterminado na abertura", "crates/capia-store/src/serverdb.rs",
      [('.execute("UPDATE idempotency SET created_ms = 0 WHERE status = 0", [])?', '.execute("SELECT 0", [])?')],
      f"{ENV} cargo test -q -p capia-server --test crash_rest -- a_pending_idempotency_key_from_a_dead_process_is_indeterminate_at_once"),
    m(36, "staging órfão não é varrido na abertura", f"{S}/core.rs",
      [('                if p.is_dir() && !p.join("meta.json").exists() {', '                if false && p.is_dir() && !p.join("meta.json").exists() {')],
      f"{ENV} cargo test -q -p capia-server --test crash_rest -- sigkill_during_an_upload"),
    m(37, "teto de tamanho de upload desligado (declarado e em streaming)", f"{S}/uploads.rs",
      [("            && n > self.cfg.max_upload_bytes\n", "            && n > u64::MAX\n"),
       ("            if size > self.cfg.max_upload_bytes {", "            if false && size > self.cfg.max_upload_bytes {")],
      f"{SR} oversized_declared_and_streamed_uploads_are_refused_and_leave_nothing"),
]


def _term(signum, frame):  # SIGTERM/SIGINT: levanta SystemExit para o `finally` restaurar o arquivo
    raise SystemExit(128 + signum)


def main() -> int:
    signal.signal(signal.SIGTERM, _term)
    signal.signal(signal.SIGINT, _term)
    want = {int(a) for a in sys.argv[1:]}
    dirty = subprocess.run(["git", "status", "--porcelain"], cwd=ROOT, capture_output=True, text=True).stdout.strip()
    if dirty:
        print("working tree is not clean; commit or stash first:\n" + dirty, file=sys.stderr)
        return 2
    results = []
    for mu in M:
        if want and mu["id"] not in want:
            continue
        path = ROOT / mu["file"]
        original = path.read_text()
        backup = path.with_suffix(path.suffix + ".mutbak")
        shutil.copy(path, backup)
        t0 = time.time()
        try:
            missing = [o for o, _ in mu["edits"] if o not in original]
            if missing:
                results.append((mu["id"], mu["name"], "PATTERN-NOT-FOUND", 0))
                print(f"[{mu['id']:>2}] PATTERN NOT FOUND in {mu['file']}: {missing[0][:70]!r}", flush=True)
                continue
            mutated = original
            for old, new in mu["edits"]:
                mutated = mutated.replace(old, new, 1)
            path.write_text(mutated)
            r = subprocess.run(mu["cmd"], shell=True, cwd=ROOT, capture_output=True, text=True)
            out = r.stdout + r.stderr
            built = "could not compile" not in out
            logs = ROOT / "target" / "mutation-phase6-logs"
            logs.mkdir(parents=True, exist_ok=True)
            (logs / f"{mu['id']}.txt").write_text("\n".join(out.splitlines()[-40:]))
            verdict = "DETECTED" if r.returncode != 0 and built else ("BROKE-BUILD" if not built else "SURVIVED")
        finally:
            path.write_text(original)
            backup.unlink(missing_ok=True)
        secs = round(time.time() - t0)
        results.append((mu["id"], mu["name"], verdict, secs))
        print(f"[{mu['id']:>2}] {verdict:<10} {mu['name']}  ({secs} s)", flush=True)
    detected = sum(1 for r in results if r[2] == "DETECTED")
    print(f"\n{detected}/{len(results)} detected")
    (ROOT / "target").mkdir(exist_ok=True)
    (ROOT / "target" / "mutation-phase6.json").write_text(json.dumps(results, indent=1))
    return 0 if detected == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
