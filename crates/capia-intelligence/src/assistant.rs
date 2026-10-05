//! Assistente de chat para **tarefas pontuais** (Fase 4): conversa, consulta o projeto por tools
//! de leitura e edita só por `timeline.preview → timeline.apply_plan` (ator `Agent`, permissões,
//! `operation_id` determinístico, auditoria). Não há Producer/Planner/Critic nem AI Run autônomo
//! (Fase 5): é um laço curto, limitado em passos, custo e tamanho de saída, 100% cancelável.
//!
//! Garantias (cada uma com teste):
//! * o modelo só enxerga as tools que a política permite; chamada fora da política é erro
//!   estruturado devolvido ao modelo (nunca execução);
//! * `apply_plan` só existe para um token **pré-visualizado nesta tarefa**; no modo `Ask` a
//!   aplicação pausa em `AwaitingApproval` e só o host (`approve`) a conclui;
//! * cancelar impede qualquer tool tardia de aplicar; o estado fica persistido como `cancelled`;
//! * conteúdo não confiável (transcrição, nomes, documentos) nunca amplia permissão.

use crate::ctx::IntelCtx;
use crate::records::{KIND_TASK, now_ms};
use crate::tools_exec::{self, Env, PlanInfo, SUPPORTED, TaskState};
use capia_ai::capability::Capability;
use capia_ai::dispatcher::{ChatOptions, StreamNotice, TaskCtx};
use capia_ai::prompt::UNTRUSTED_PREAMBLE;
use capia_ai::router::RouteRequest;
use capia_ai::tools::{
    AuditEntry, Gates, ToolError, ToolErrorCode, ToolPolicy, ToolRegistry, bound_output,
    input_digest, phase4_registry, validate_result,
};
use capia_ai::types::{ChatEvent, ChatRequest, Message, Part, Role, ToolCallOut};
use capia_ai::usage::BudgetCheck;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const MAX_STEPS: u32 = 8;
pub const MAX_TOOL_CALLS_PER_STEP: usize = 4;
pub const MAX_TOOL_OUTPUT_BYTES: usize = 16_000;
const MAX_HISTORY_MESSAGES: usize = 24;
const MAX_MESSAGE_CHARS: usize = 8_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    /// Pausa antes de aplicar e pede aprovação (padrão).
    Ask,
    /// Aplica sozinho — ainda por `preview → apply_plan` (a política configurada pelo usuário).
    Auto,
}

#[derive(Clone, Debug)]
pub struct AssistantOptions {
    pub mode: ApprovalMode,
    pub max_steps: u32,
}

impl Default for AssistantOptions {
    fn default() -> Self {
        Self {
            mode: ApprovalMode::Ask,
            max_steps: MAX_STEPS,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum AssistantEvent {
    Text(String),
    /// Descartar o texto parcial (nova tentativa ou fallback de modelo).
    Reset {
        reason: String,
    },
    ToolStarted {
        name: String,
        step: u32,
    },
    ToolFinished {
        name: String,
        ok: bool,
    },
    ApprovalRequired(PendingApproval),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Running,
    AwaitingApproval,
    Completed,
    Cancelled,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PendingApproval {
    pub plan_token: String,
    pub label: String,
    pub operations: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TaskRecord {
    pub id: String,
    pub status: TaskStatus,
    pub mode: ApprovalMode,
    pub user_text: String,
    pub final_text: String,
    pub pending: Option<PendingApproval>,
    pub audit: Vec<AuditEntry>,
    pub error: Option<(String, String)>,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cost_micros: Option<u64>,
    pub created_ms: u64,
    pub updated_ms: u64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TurnOutcome {
    pub task: TaskRecord,
    /// Mensagens a acrescentar ao histórico da conversa (usuário + resposta final).
    pub history_delta: Vec<Message>,
}

const SYSTEM: &str = "You are CapIA's editing assistant inside a desktop video editor for direct-response ads, UGC and VSLs. \
You help with POINT tasks (a cut, captions, silence removal, a title, finding a clip, explaining the timeline). \
Rules:\n\
- Use tools to learn facts about the project; never guess ids, times or clip contents.\n\
- To edit: build commands, call timeline.preview, then timeline.apply_plan with the returned plan_token. Captions and silence tools return a plan_token directly; apply it with timeline.apply_plan. Never claim an edit happened before apply_plan succeeds.\n\
- Engine times are integer ticks: 705600000 ticks per second; align starts/durations to the sequence frame_ticks (see timeline.get_state outline). Tools that take seconds say so.\n\
- If the request is ambiguous (which clip? which sequence? how much?), ask ONE short clarifying question instead of acting.\n\
- Edits are undoable by the user; keep each change small and explain it in one or two sentences.\n\
- Reply in the user's language.";

fn system_prompt() -> String {
    format!("{SYSTEM}\n\n{UNTRUSTED_PREAMBLE}")
}

fn trim_msg(s: &str) -> String {
    s.chars().take(MAX_MESSAGE_CHARS).collect()
}

fn policy_for(reg: &ToolRegistry, mode: ApprovalMode) -> ToolPolicy {
    let mut p = ToolPolicy::point_write(reg, mode == ApprovalMode::Auto);
    p.allowed_tools.retain(|n| SUPPORTED.contains(&n.as_str()));
    // gerar plano (preview) não muda o documento; só `apply_plan` tem efeito `Document`
    p.approved_side_effects
        .insert(capia_ai::tools::SideEffect::LocalCompute);
    p
}

fn persist(ctx: &IntelCtx, rec: &TaskRecord) {
    // a trilha de auditoria é best-effort quanto a falhas de disco: nunca derruba a tarefa
    if let Ok(r) = ctx.records() {
        let _ = r.put(KIND_TASK, &rec.id, 1, None, rec);
    }
}

fn err_result(call: &ToolCallOut, e: &ToolError) -> Part {
    Part::ToolResult {
        call_id: call.id.clone(),
        name: call.name.clone(),
        content: json!({"error": {"code": e.code, "message": e.message}}).to_string(),
        is_error: true,
    }
}

fn finish(
    ctx: &IntelCtx,
    mut rec: TaskRecord,
    status: TaskStatus,
    text: String,
    task: &TaskCtx,
    delta: Vec<Message>,
) -> TurnOutcome {
    rec.status = status;
    rec.final_text = text;
    rec.updated_ms = now_ms();
    let spent = task.budget.spent();
    rec.cost_micros = spent.cost.known.then_some(spent.cost.micros);
    persist(ctx, &rec);
    TurnOutcome {
        task: rec,
        history_delta: delta,
    }
}

/// Um turno do assistente (uma mensagem do usuário → resposta final, aprovação pendente ou erro).
pub async fn run_turn(
    ctx: &IntelCtx,
    task: &TaskCtx,
    history: &[Message],
    user_text: &str,
    opts: &AssistantOptions,
    on: &(dyn Fn(AssistantEvent) + Send + Sync),
) -> TurnOutcome {
    let registry = phase4_registry();
    let policy = policy_for(&registry, opts.mode);
    let mut state = TaskState::default();
    {
        let mut vr = RouteRequest::for_capability(Capability::VisionInput);
        vr.data.push(capia_ai::brain::DataClass::Frames);
        state.vision_ok = ctx.ai.route_preview(&vr).is_ok();
    }
    let mut rec = TaskRecord {
        id: task.task_id.clone(),
        status: TaskStatus::Running,
        mode: opts.mode,
        user_text: trim_msg(user_text),
        final_text: String::new(),
        pending: None,
        audit: Vec::new(),
        error: None,
        input_tokens: 0,
        output_tokens: 0,
        cost_micros: None,
        created_ms: now_ms(),
        updated_ms: now_ms(),
    };
    persist(ctx, &rec);

    let mut messages: Vec<Message> = vec![Message::system(system_prompt())];
    let skip = history.len().saturating_sub(MAX_HISTORY_MESSAGES);
    for m in &history[skip..] {
        // só texto de usuário/assistente das rodadas anteriores (nunca resultados de tools antigos)
        if matches!(m.role, Role::User | Role::Assistant) {
            let t = trim_msg(&m.text_of());
            if !t.is_empty() {
                messages.push(Message {
                    role: m.role.clone(),
                    parts: vec![Part::text(t)],
                });
            }
        }
    }
    let user_msg = Message::user(trim_msg(user_text));
    messages.push(user_msg.clone());
    let mut final_text = String::new();
    let mut repeated_errors = 0u32;

    for step in 0..opts.max_steps {
        if task.cancel.is_cancelled() {
            return finish(
                ctx,
                rec,
                TaskStatus::Cancelled,
                final_text,
                task,
                vec![user_msg],
            );
        }
        if let BudgetCheck::Exceeded(m) = task.budget.check(None) {
            rec.error = Some(("BUDGET_EXCEEDED".into(), m));
            return finish(
                ctx,
                rec,
                TaskStatus::Failed,
                final_text,
                task,
                vec![user_msg],
            );
        }
        let mut req = ChatRequest::new(String::new(), messages.clone());
        req.tools = registry.specs_for(&policy);
        req.meta.task_id = Some(task.task_id.clone());
        req.meta.purpose = Some("assistant".into());
        let cb = |n: StreamNotice| match n {
            StreamNotice::Event(ChatEvent::TextDelta { text }) => on(AssistantEvent::Text(text)),
            StreamNotice::Retry { reason, .. } | StreamNotice::Fallback { reason, .. } => {
                on(AssistantEvent::Reset { reason });
            }
            StreamNotice::Event(_) => {}
        };
        let mut route = RouteRequest::for_capability(Capability::TextGeneration);
        route.data.push(capia_ai::brain::DataClass::Text);
        let out = ctx
            .ai
            .chat(
                task,
                req,
                ChatOptions {
                    route,
                    cacheable: false,
                },
                Some(&cb),
            )
            .await;
        let out = match out {
            Ok(o) => o,
            Err(e) => {
                let status = if e.code == capia_ai::ErrorCode::Cancelled {
                    TaskStatus::Cancelled
                } else {
                    rec.error = Some((e.code.as_str().to_owned(), e.message.clone()));
                    TaskStatus::Failed
                };
                return finish(ctx, rec, status, final_text, task, vec![user_msg]);
            }
        };
        rec.input_tokens += out.response.usage.input_tokens;
        rec.output_tokens += out.response.usage.output_tokens;
        final_text.clone_from(&out.response.text);
        let calls = out.response.tool_calls.clone();
        let mut parts: Vec<Part> = Vec::new();
        if !out.response.text.is_empty() {
            parts.push(Part::text(out.response.text.clone()));
        }
        for c in &calls {
            parts.push(Part::ToolCall {
                id: c.id.clone(),
                name: c.name.clone(),
                arguments: c.arguments.clone(),
            });
        }
        messages.push(Message {
            role: Role::Assistant,
            parts,
        });
        if calls.is_empty() {
            let delta = vec![user_msg, Message::assistant(trim_msg(&out.response.text))];
            return finish(
                ctx,
                rec,
                TaskStatus::Completed,
                out.response.text,
                task,
                delta,
            );
        }

        let mut results: Vec<Part> = Vec::new();
        for (idx, call) in calls.iter().enumerate() {
            if idx >= MAX_TOOL_CALLS_PER_STEP {
                results.push(err_result(
                    call,
                    &ToolError::new(
                        ToolErrorCode::Failed,
                        format!("at most {MAX_TOOL_CALLS_PER_STEP} tool calls per step"),
                    ),
                ));
                continue;
            }
            // cancelamento: nenhuma tool tardia roda nem aplica
            if task.cancel.is_cancelled() {
                return finish(
                    ctx,
                    rec,
                    TaskStatus::Cancelled,
                    final_text,
                    task,
                    vec![user_msg],
                );
            }
            let started = now_ms();
            let gates = Gates {
                budget_ok: !matches!(task.budget.check(None), BudgetCheck::Exceeded(_)),
                privacy_ok: true,
            };
            let mut entry = AuditEntry {
                task_id: task.task_id.clone(),
                step,
                tool: call.name.clone(),
                version: 1,
                permission: None,
                side_effect: None,
                status: "denied".into(),
                error: None,
                input_digest: input_digest(&call.arguments),
                output_bytes: 0,
                started_ms: started,
                finished_ms: started,
                transaction_ref: None,
            };
            let def = match registry.authorize(&policy, call, gates) {
                Ok(d) => d,
                Err(e)
                    if e.code == ToolErrorCode::SideEffectNotApproved
                        && call.name == "timeline.apply_plan" =>
                {
                    // modo Ask: pausa e entrega o plano ao usuário
                    let token = call.arguments["plan_token"]
                        .as_str()
                        .unwrap_or("")
                        .to_owned();
                    let Some(PlanInfo {
                        label, op_count, ..
                    }) = state.plans.get(&token).cloned()
                    else {
                        let e2 = ToolError::new(
                            ToolErrorCode::InvalidArguments,
                            "unknown plan_token for this task: call timeline.preview first",
                        );
                        entry.error = Some(e2.message.clone());
                        rec.audit.push(entry);
                        results.push(err_result(call, &e2));
                        continue;
                    };
                    let pending = PendingApproval {
                        plan_token: token,
                        label,
                        operations: op_count,
                    };
                    entry.status = "awaiting_approval".into();
                    entry.permission = Some(capia_ai::tools::Permission::WriteTimeline);
                    entry.side_effect = Some(capia_ai::tools::SideEffect::Document);
                    entry.finished_ms = now_ms();
                    rec.audit.push(entry);
                    rec.pending = Some(pending.clone());
                    on(AssistantEvent::ApprovalRequired(pending));
                    let delta = vec![user_msg, Message::assistant(trim_msg(&final_text))];
                    return finish(
                        ctx,
                        rec,
                        TaskStatus::AwaitingApproval,
                        final_text,
                        task,
                        delta,
                    );
                }
                Err(e) => {
                    entry.error = Some(format!("{:?}: {}", e.code, e.message));
                    entry.finished_ms = now_ms();
                    rec.audit.push(entry);
                    results.push(err_result(call, &e));
                    repeated_errors += 1;
                    if repeated_errors >= 3 {
                        rec.error = Some((
                            "TOOL_CALL_INVALID".into(),
                            "the model kept calling tools that are not allowed".into(),
                        ));
                        return finish(
                            ctx,
                            rec,
                            TaskStatus::Failed,
                            final_text,
                            task,
                            vec![user_msg],
                        );
                    }
                    continue;
                }
            };
            entry.permission = Some(def.permission);
            entry.side_effect = Some(def.side_effect);
            let timeout = std::time::Duration::from_millis(def.timeout_ms.max(1_000));
            on(AssistantEvent::ToolStarted {
                name: call.name.clone(),
                step,
            });
            let exec = {
                let mut env = Env {
                    ctx,
                    task,
                    step,
                    state: &mut state,
                };
                tokio::time::timeout(
                    timeout,
                    tools_exec::execute(&call.name, &call.arguments, &mut env),
                )
                .await
            };
            // cancelado durante a execução: o resultado é descartado (não é entregue ao modelo)
            if task.cancel.is_cancelled() {
                entry.status = "cancelled".into();
                entry.finished_ms = now_ms();
                rec.audit.push(entry);
                return finish(
                    ctx,
                    rec,
                    TaskStatus::Cancelled,
                    final_text,
                    task,
                    vec![user_msg],
                );
            }
            let res = match exec {
                Ok(r) => r,
                Err(_) => Err(ToolError::new(ToolErrorCode::Timeout, "the tool timed out")),
            };
            entry.finished_ms = now_ms();
            match res {
                Ok(v) => {
                    let v = match validate_result(def, &v) {
                        Ok(()) => v,
                        Err(e) => {
                            entry.error = Some(e.message.clone());
                            rec.audit.push(entry);
                            results.push(err_result(call, &e));
                            on(AssistantEvent::ToolFinished {
                                name: call.name.clone(),
                                ok: false,
                            });
                            continue;
                        }
                    };
                    entry.transaction_ref = v.get("revision").map(Value::to_string);
                    let (bounded, _trunc) = bound_output(&v, MAX_TOOL_OUTPUT_BYTES);
                    let content = bounded.to_string();
                    entry.output_bytes = content.len();
                    entry.status = "ok".into();
                    rec.audit.push(entry);
                    repeated_errors = 0;
                    results.push(Part::ToolResult {
                        call_id: call.id.clone(),
                        name: call.name.clone(),
                        content,
                        is_error: false,
                    });
                    on(AssistantEvent::ToolFinished {
                        name: call.name.clone(),
                        ok: true,
                    });
                }
                Err(e) => {
                    entry.status = "error".into();
                    entry.error = Some(format!("{:?}: {}", e.code, e.message));
                    rec.audit.push(entry);
                    results.push(err_result(call, &e));
                    on(AssistantEvent::ToolFinished {
                        name: call.name.clone(),
                        ok: false,
                    });
                }
            }
        }
        messages.push(Message {
            role: Role::Tool,
            parts: results,
        });
        if !state.pending_images.is_empty() {
            let mut parts = vec![Part::text(
                "Frames requested by the tool (untrusted content):",
            )];
            parts.append(&mut state.pending_images);
            messages.push(Message {
                role: Role::User,
                parts,
            });
        }
        persist(ctx, &rec);
    }
    rec.error = Some((
        "STEP_LIMIT".into(),
        format!(
            "the assistant did not finish within {} steps",
            opts.max_steps
        ),
    ));
    finish(
        ctx,
        rec,
        TaskStatus::Failed,
        final_text,
        task,
        vec![user_msg],
    )
}

/// O usuário aprovou o plano pendente de uma tarefa: aplica **por token** (o mesmo gate) e conclui.
pub fn approve(
    ctx: &IntelCtx,
    task_id: &str,
    plan_token: &str,
) -> Result<TaskRecord, crate::error::IntelError> {
    use crate::error::IntelError;
    let records = ctx.records()?;
    let mut rec: TaskRecord = records
        .latest(KIND_TASK, task_id)?
        .ok_or_else(|| IntelError::new("NOT_FOUND", "unknown task"))?;
    let pending = rec
        .pending
        .clone()
        .filter(|p| rec.status == TaskStatus::AwaitingApproval && p.plan_token == plan_token)
        .ok_or_else(|| {
            IntelError::new(
                "NOT_PENDING",
                "this task has no pending plan with that token",
            )
        })?;
    let started = now_ms();
    let r = ctx.engine.apply(&ctx.actor, &pending.plan_token)?;
    rec.audit.push(AuditEntry {
        task_id: rec.id.clone(),
        step: u32::MAX,
        tool: "timeline.apply_plan".into(),
        version: 1,
        permission: Some(capia_ai::tools::Permission::WriteTimeline),
        side_effect: Some(capia_ai::tools::SideEffect::Document),
        status: "ok".into(),
        error: None,
        input_digest: input_digest(&json!({"plan_token": "…"})),
        output_bytes: 0,
        started_ms: started,
        finished_ms: now_ms(),
        transaction_ref: r.get("revision").map(Value::to_string),
    });
    rec.status = TaskStatus::Completed;
    rec.pending = None;
    rec.final_text = format!(
        "{} — applied ({} operations). Undo with Ctrl+Z.",
        pending.label, pending.operations
    );
    rec.updated_ms = now_ms();
    records.put(KIND_TASK, &rec.id, 1, None, &rec)?;
    Ok(rec)
}

/// O usuário recusou: nada é aplicado.
pub fn reject(ctx: &IntelCtx, task_id: &str) -> Result<TaskRecord, crate::error::IntelError> {
    use crate::error::IntelError;
    let records = ctx.records()?;
    let mut rec: TaskRecord = records
        .latest(KIND_TASK, task_id)?
        .ok_or_else(|| IntelError::new("NOT_FOUND", "unknown task"))?;
    if rec.status != TaskStatus::AwaitingApproval {
        return Err(IntelError::new(
            "NOT_PENDING",
            "the task has no pending plan",
        ));
    }
    rec.status = TaskStatus::Cancelled;
    rec.pending = None;
    rec.final_text = "Plan rejected; nothing was applied.".into();
    rec.updated_ms = now_ms();
    records.put(KIND_TASK, &rec.id, 1, None, &rec)?;
    Ok(rec)
}
