import { useEffect, useState } from "react";
import { Badge, Button, Dialog, Select, TextInput } from "@capia/ui-kit";
import type {
  AiModelEndpoint,
  AiProviderView,
  BrainProfileView,
  ProviderKind,
} from "@capia/engine-bindings";
import { useAi, useController } from "../context";
import { useT, type MessageKey } from "../i18n";

const CAPS = [
  "text_generation",
  "streaming",
  "tool_calling",
  "structured_output",
  "vision_input",
  "speech_to_text",
] as const;

const NO_MODELS: AiModelEndpoint[] = [];
const NO_PROFILES: BrainProfileView[] = [];

type CapRecord = Record<string, { supported?: boolean; origin?: string }>;

function capsOf(m: AiModelEndpoint): CapRecord {
  // o engine serializa `Capabilities` como `{ entries: { <capability>: { supported, origin } } }`
  const raw = m.capabilities as { entries?: CapRecord } | null;
  return raw?.entries ?? {};
}

/** Providers, modelos, Brain Profile, privacidade, diagnóstico e uso. A chave é write-only. */
export function AiSettingsDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const c = useController();
  const t = useT();
  const { status, diagnostics, usage } = useAi((s) => ({
    status: s.status,
    diagnostics: s.diagnostics,
    usage: s.usage,
  }));
  useEffect(() => {
    if (open) {
      void c.ai.refresh();
      void c.ai.loadUsage();
    }
  }, [open, c]);
  const memoryBackend = status?.secret_backend === "memory";
  return (
    <Dialog
      title={t("ai.settings.title")}
      open={open}
      onClose={onClose}
      footer={<Button onClick={onClose}>{t("common.close")}</Button>}
    >
      <div className="ed-settings ed-ai-settings" data-testid="ai-settings">
        <label className="ed-row">
          <input
            type="checkbox"
            data-testid="ai-enabled"
            checked={status?.enabled ?? true}
            onChange={(e) => {
              void c.ai.setEnabled(e.currentTarget.checked);
            }}
          />
          <span>{t("ai.enabled")}</span>
        </label>
        {status && (
          <p className="ed-hint" data-testid="ai-backend">
            {t("ai.backend", { backend: status.secret_backend })}
            {memoryBackend ? ` — ${t("ai.backendMemory")}` : ""}
          </p>
        )}
        <h3 className="ed-subhead">{t("ai.providers")}</h3>
        <ul className="ed-ai-providers" data-testid="ai-providers">
          {(status?.providers ?? []).map((p) => (
            <ProviderRow key={p.id} p={p} />
          ))}
        </ul>
        <AddProvider />
        <h3 className="ed-subhead">{t("ai.models")}</h3>
        <ul className="ed-ai-models" data-testid="ai-models">
          {(status?.models ?? []).map((m) => (
            <ModelRow key={m.id} m={m} />
          ))}
        </ul>
        <AddModel />
        <BrainSection />
        <h3 className="ed-subhead">{t("ai.usage")}</h3>
        {usage && (
          <p data-testid="ai-usage">
            {t("ai.usage.line", {
              calls: usage.calls,
              input: usage.input_tokens,
              output: usage.output_tokens,
              cost:
                usage.unknown_cost_calls > 0 && usage.known_cost_micros === 0
                  ? t("ai.usage.unknownCost")
                  : `${(usage.known_cost_micros / 1e6).toFixed(4)} ${usage.currency ?? ""}`.trim(),
            })}
          </p>
        )}
        <div className="ed-row">
          <Button
            data-testid="ai-diagnostics"
            onClick={() => {
              void c.ai.loadDiagnostics();
            }}
          >
            {t("ai.diagnostics")}
          </Button>
        </div>
        {diagnostics && (
          <textarea
            readOnly
            data-testid="ai-diagnostics-text"
            aria-label={t("ai.diagnostics")}
            rows={6}
            value={diagnostics}
          />
        )}
      </div>
    </Dialog>
  );
}

function ProviderRow({ p }: { p: AiProviderView }) {
  const c = useController();
  const t = useT();
  return (
    <li data-testid={`ai-provider-${p.id}`}>
      <strong>{p.display_name}</strong> <small>{p.kind}</small>{" "}
      {p.base_url && <small>{p.base_url}</small>}
      {p.local && <Badge>{t("ai.provider.local")}</Badge>}
      {p.needs_credential && (
        <Badge
          tone={p.credential_configured ? "success" : "warning"}
          data-testid={`ai-cred-${p.id}`}
        >
          {p.credential_configured
            ? t("ai.provider.credentialSet")
            : t("ai.provider.credentialMissing")}
        </Badge>
      )}
      <div className="ed-row">
        <label className="ed-row">
          <input
            type="checkbox"
            checked={p.enabled}
            onChange={(e) => {
              void c.ai.saveProvider({
                id: p.id,
                kind: p.kind,
                display_name: p.display_name,
                base_url: p.base_url,
                enabled: e.currentTarget.checked,
                allow_loopback: p.allow_loopback,
              });
            }}
          />
          <span>{t("ai.provider.enabled")}</span>
        </label>
        <Button
          variant="ghost"
          onClick={() => {
            void c.ai.importModels(p.id);
          }}
        >
          {t("ai.provider.import")}
        </Button>
        {p.credential_configured && (
          <Button
            variant="ghost"
            data-testid={`ai-rmkey-${p.id}`}
            onClick={() => {
              void c.ai.deleteCredential(p.id);
            }}
          >
            {t("ai.provider.removeKey")}
          </Button>
        )}
        <Button
          variant="ghost"
          data-testid={`ai-rmprov-${p.id}`}
          onClick={() => {
            void c.ai.deleteProvider(p.id);
          }}
        >
          {t("ai.provider.delete")}
        </Button>
      </div>
    </li>
  );
}

const KINDS: ProviderKind[] = [
  "open_ai_compatible",
  "anthropic",
  "google",
  "local_open_ai_compatible",
  "whisper_local",
];

function AddProvider() {
  const c = useController();
  const t = useT();
  const presets = useAi((s) => s.status?.presets ?? []);
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ProviderKind>("open_ai_compatible");
  const [baseUrl, setBaseUrl] = useState("");
  const [loopback, setLoopback] = useState(false);
  // a chave vive só neste campo controlado e é apagada logo após o envio
  const [apiKey, setApiKey] = useState("");
  const local = kind === "local_open_ai_compatible" || kind === "whisper_local";
  const valid = /^[A-Za-z0-9_-]{1,64}$/.test(id) && name.trim() !== "";
  return (
    <div className="ed-ai-form" data-testid="ai-add-provider">
      <h4 className="ed-subhead">{t("ai.provider.add")}</h4>
      <Select
        label={t("ai.provider.preset")}
        value=""
        data-testid="ai-preset"
        onChange={(e) => {
          const pr = presets.find((x) => x.key === e.currentTarget.value);
          if (!pr) return;
          setId(pr.key);
          setName(pr.display_name);
          setKind(pr.kind);
          setBaseUrl(pr.base_url ?? "");
          setLoopback(pr.kind === "local_open_ai_compatible" || pr.kind === "whisper_local");
        }}
      >
        <option value="">{t("ai.provider.custom")}</option>
        {presets.map((p) => (
          <option key={p.key} value={p.key}>
            {p.display_name}
          </option>
        ))}
      </Select>
      <TextInput
        label={t("ai.provider.id")}
        value={id}
        data-testid="ai-prov-id"
        onChange={(e) => {
          setId(e.currentTarget.value);
        }}
      />
      <TextInput
        label={t("ai.provider.name")}
        value={name}
        data-testid="ai-prov-name"
        onChange={(e) => {
          setName(e.currentTarget.value);
        }}
      />
      <Select
        label={t("ai.provider.kind")}
        value={kind}
        data-testid="ai-prov-kind"
        onChange={(e) => {
          setKind(e.currentTarget.value as ProviderKind);
        }}
      >
        {KINDS.map((k) => (
          <option key={k} value={k}>
            {k}
          </option>
        ))}
      </Select>
      <TextInput
        label={t("ai.provider.baseUrl")}
        value={baseUrl}
        data-testid="ai-prov-url"
        onChange={(e) => {
          setBaseUrl(e.currentTarget.value);
        }}
      />
      {local && (
        <label className="ed-row">
          <input
            type="checkbox"
            checked={loopback}
            onChange={(e) => {
              setLoopback(e.currentTarget.checked);
            }}
          />
          <span>{t("ai.provider.allowLoopback")}</span>
        </label>
      )}
      <TextInput
        label={t("ai.provider.apiKey")}
        type="password"
        autoComplete="off"
        spellCheck={false}
        value={apiKey}
        data-testid="ai-prov-key"
        onChange={(e) => {
          setApiKey(e.currentTarget.value);
        }}
      />
      <p className="ed-hint">{t("ai.provider.apiKeyHint")}</p>
      <Button
        variant="primary"
        disabled={!valid}
        data-testid="ai-prov-save"
        onClick={() => {
          const key = apiKey;
          setApiKey("");
          void c.ai
            .saveProvider(
              {
                id,
                kind,
                display_name: name.trim(),
                base_url: baseUrl.trim() || null,
                enabled: true,
                allow_loopback: loopback,
              },
              key || undefined,
            )
            .then((ok) => {
              if (ok) {
                setId("");
                setName("");
                setBaseUrl("");
              }
            });
        }}
      >
        {t("ai.provider.save")}
      </Button>
    </div>
  );
}

function ModelRow({ m }: { m: AiModelEndpoint }) {
  const c = useController();
  const t = useT();
  const caps = capsOf(m);
  const probe = m.last_probe;
  return (
    <li data-testid={`ai-model-${m.id}`}>
      <strong>{m.model_id}</strong> <small>{m.provider_id}</small>{" "}
      <Badge>{t("ai.model.health", { state: m.health })}</Badge>
      <div className="ed-ai-caps">
        {CAPS.map((k) => {
          const cap = caps[k];
          if (!cap?.supported) return null;
          const origin = (cap.origin ?? "declared") as "declared" | "probed" | "preset";
          return (
            <Badge key={k} {...(origin === "probed" ? { tone: "success" as const } : {})}>
              {t(`ai.cap.${k}` as MessageKey)} · {t(`ai.model.origin.${origin}` as MessageKey)}
            </Badge>
          );
        })}
      </div>
      {probe && (
        <small data-testid={`ai-probe-${m.id}`}>
          {t("ai.model.lastProbe", {
            result: probe.success ? t("ai.model.probeOk") : t("ai.model.probeFail"),
            ms: probe.latency_ms,
          })}
        </small>
      )}
      <div className="ed-row">
        <Button
          data-testid={`ai-probe-btn-${m.id}`}
          onClick={() => {
            void c.ai.probe(m.id);
          }}
        >
          {t("ai.model.probe")}
        </Button>
        <Button
          variant="ghost"
          onClick={() => {
            void c.ai.deleteModel(m.id);
          }}
        >
          {t("common.delete")}
        </Button>
      </div>
    </li>
  );
}

const DECLARABLE: { id: string; key: (typeof CAPS)[number] }[] = [
  { id: "text", key: "text_generation" },
  { id: "streaming", key: "streaming" },
  { id: "tools", key: "tool_calling" },
  { id: "struct", key: "structured_output" },
  { id: "vision", key: "vision_input" },
  { id: "stt", key: "speech_to_text" },
];

function AddModel() {
  const c = useController();
  const t = useT();
  const providers = useAi((s) => s.status?.providers ?? []);
  const [provider, setProvider] = useState("");
  const [modelId, setModelId] = useState("");
  const [ctx, setCtx] = useState("128000");
  const [caps, setCaps] = useState<Record<string, boolean>>({ text: true, streaming: true });
  const prov = provider || providers[0]?.id || "";
  const valid = prov !== "" && modelId.trim() !== "";
  return (
    <div className="ed-ai-form" data-testid="ai-add-model">
      <h4 className="ed-subhead">{t("ai.model.add")}</h4>
      <Select
        label={t("ai.model.provider")}
        value={prov}
        data-testid="ai-model-provider"
        onChange={(e) => {
          setProvider(e.currentTarget.value);
        }}
      >
        {providers.map((p) => (
          <option key={p.id} value={p.id}>
            {p.display_name}
          </option>
        ))}
      </Select>
      <TextInput
        label={t("ai.model.id")}
        value={modelId}
        data-testid="ai-model-id"
        onChange={(e) => {
          setModelId(e.currentTarget.value);
        }}
      />
      <TextInput
        label={t("ai.model.context")}
        value={ctx}
        inputMode="numeric"
        onChange={(e) => {
          setCtx(e.currentTarget.value.replace(/\D/g, ""));
        }}
      />
      <div className="ed-ai-caps">
        {DECLARABLE.map((d) => (
          <label key={d.id} className="ed-row">
            <input
              type="checkbox"
              data-testid={`ai-cap-${d.id}`}
              checked={caps[d.id] === true}
              onChange={(e) => {
                setCaps((x) => ({ ...x, [d.id]: e.currentTarget.checked }));
              }}
            />
            <span>{t(`ai.cap.${d.key}` as MessageKey)}</span>
          </label>
        ))}
      </div>
      <Button
        variant="primary"
        disabled={!valid}
        data-testid="ai-model-save"
        onClick={() => {
          const declared = DECLARABLE.filter((d) => caps[d.id]).map((d) => d.key);
          void c.ai
            .saveModel({
              id: `${prov}:${modelId.trim()}`,
              provider_id: prov,
              model_id: modelId.trim(),
              display_name: modelId.trim(),
              enabled: true,
              context_window: Number(ctx) || 0,
              max_output_tokens: 0,
              capabilities: declaredCaps(declared),
            })
            .then((ok) => {
              if (ok) setModelId("");
            });
        }}
      >
        {t("ai.model.save")}
      </Button>
    </div>
  );
}

/** Formato serializado de `Capabilities` do engine (origem = declarada pelo usuário). */
function declaredCaps(keys: string[]): Record<string, unknown> {
  const entries: Record<string, { supported: boolean; origin: string }> = {};
  for (const k of keys) entries[k] = { supported: true, origin: "declared" };
  return { entries };
}

function BrainSection() {
  const { profiles, active } = useAi((s) => ({
    profiles: s.status?.profiles ?? NO_PROFILES,
    active: s.status?.active_profile ?? null,
  }));
  const current = profiles.find((p) => p.id === active);
  // remonta o formulário quando o perfil ativo muda (estado inicial vem das props, sem efeito)
  return <BrainForm key={`${current?.id ?? "none"}:${current?.brain ?? ""}`} current={current} />;
}

function BrainForm({ current }: { current: BrainProfileView | undefined }) {
  const c = useController();
  const t = useT();
  const models = useAi((s) => s.status?.models ?? NO_MODELS);
  const [brain, setBrain] = useState(current?.brain ?? "");
  const [fallback, setFallback] = useState(current?.fallbacks?.text_generation?.[0] ?? "");
  const [localStt, setLocalStt] = useState(current?.privacy?.transcription_local_only ?? false);
  const [noAudio, setNoAudio] = useState(current?.privacy?.no_cloud_audio ?? false);
  const [noDocs, setNoDocs] = useState(current?.privacy?.no_external_document_upload ?? false);
  const [maxCost, setMaxCost] = useState(String(current?.budgets?.max_cost_per_task_micros ?? ""));
  const selected = brain || models[0]?.id || "";
  return (
    <div className="ed-ai-form" data-testid="ai-brain">
      <h3 className="ed-subhead">{t("ai.brain")}</h3>
      <Select
        label={t("ai.brain.primary")}
        value={selected}
        data-testid="ai-brain-select"
        onChange={(e) => {
          setBrain(e.currentTarget.value);
        }}
      >
        {models.map((m) => (
          <option key={m.id} value={m.id}>
            {m.provider_id} · {m.model_id}
          </option>
        ))}
      </Select>
      <Select
        label={t("ai.brain.fallback")}
        value={fallback}
        data-testid="ai-fallback-select"
        onChange={(e) => {
          setFallback(e.currentTarget.value);
        }}
      >
        <option value="">{t("common.none")}</option>
        {models
          .filter((m) => m.id !== selected)
          .map((m) => (
            <option key={m.id} value={m.id}>
              {m.provider_id} · {m.model_id}
            </option>
          ))}
      </Select>
      <h4 className="ed-subhead">{t("ai.privacy")}</h4>
      {(
        [
          ["ai.privacy.transcriptionLocalOnly", localStt, setLocalStt, "ai-priv-local"],
          ["ai.privacy.noCloudAudio", noAudio, setNoAudio, "ai-priv-audio"],
          ["ai.privacy.noExternalDocs", noDocs, setNoDocs, "ai-priv-docs"],
        ] as const
      ).map(([key, val, set, tid]) => (
        <label key={key} className="ed-row">
          <input
            type="checkbox"
            data-testid={tid}
            checked={val}
            onChange={(e) => {
              set(e.currentTarget.checked);
            }}
          />
          <span>{t(key)}</span>
        </label>
      ))}
      <TextInput
        label={t("ai.budget.maxCost")}
        value={maxCost}
        inputMode="numeric"
        onChange={(e) => {
          setMaxCost(e.currentTarget.value.replace(/\D/g, ""));
        }}
      />
      <Button
        variant="primary"
        disabled={selected === ""}
        data-testid="ai-brain-save"
        onClick={() => {
          void c.ai.setBrain({
            id: current?.id ?? "default",
            name: current?.name ?? "Default",
            brain: selected,
            role_overrides: current?.role_overrides ?? {},
            fallbacks: fallback ? { text_generation: [fallback] } : {},
            budgets: {
              ...(current?.budgets ?? {}),
              max_cost_per_task_micros: maxCost ? Number(maxCost) : null,
            },
            privacy: {
              never_upload_video: true,
              vision_frames_only: true,
              transcription_local_only: localStt,
              no_cloud_audio: noAudio,
              no_external_document_upload: noDocs,
            },
          });
        }}
      >
        {t("ai.brain.save")}
      </Button>
    </div>
  );
}
