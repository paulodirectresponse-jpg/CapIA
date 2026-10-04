import {
  type ButtonHTMLAttributes,
  type InputHTMLAttributes,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type SelectHTMLAttributes,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { Icon, type IconName } from "./icons";

// ------------------------------------------------------------------------------------ botões

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "default" | "primary" | "danger" | "ghost";
}

export function Button({ variant = "default", className, type = "button", ...rest }: ButtonProps) {
  return (
    <button
      type={type}
      className={["cp-btn", className].filter(Boolean).join(" ")}
      data-variant={variant}
      {...rest}
    />
  );
}

export interface IconButtonProps extends Omit<ButtonProps, "children" | "aria-label"> {
  icon: IconName;
  /** Obrigatório: nome acessível e tooltip (controles só-ícone). */
  label: string;
  pressed?: boolean;
  shortcut?: string;
}

export function IconButton({
  icon,
  label,
  pressed,
  shortcut,
  className,
  ...rest
}: IconButtonProps) {
  const tip = shortcut ? `${label} (${shortcut})` : label;
  return (
    <Tooltip text={tip}>
      <Button
        className={["cp-iconbtn", className].filter(Boolean).join(" ")}
        aria-label={label}
        aria-pressed={pressed}
        variant="ghost"
        {...rest}
      >
        <Icon name={icon} />
      </Button>
    </Tooltip>
  );
}

// ------------------------------------------------------------------------------------ campos

export interface TextInputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  invalid?: boolean;
}

export function TextInput({ label, invalid, className, id, ...rest }: TextInputProps) {
  const auto = useId();
  const inputId = id ?? auto;
  const input = (
    <input
      id={inputId}
      className={["cp-input", className].filter(Boolean).join(" ")}
      aria-invalid={invalid ? true : undefined}
      {...rest}
    />
  );
  if (!label) return input;
  return (
    <label className="cp-field" htmlFor={inputId}>
      <span>{label}</span>
      {input}
    </label>
  );
}

export interface SelectProps extends SelectHTMLAttributes<HTMLSelectElement> {
  label?: string;
}

export function Select({ label, className, children, id, ...rest }: SelectProps) {
  const auto = useId();
  const selectId = id ?? auto;
  const el = (
    <select id={selectId} className={["cp-select", className].filter(Boolean).join(" ")} {...rest}>
      {children}
    </select>
  );
  if (!label) return el;
  return (
    <label className="cp-field" htmlFor={selectId}>
      <span>{label}</span>
      {el}
    </label>
  );
}

export interface SliderProps {
  value: number;
  min: number;
  max: number;
  step?: number;
  label: string;
  onChange: (value: number) => void;
  /** Disparado ao soltar (commit único; o `onChange` serve ao preview local). */
  onCommit?: (value: number) => void;
}

export function Slider({ value, min, max, step = 1, label, onChange, onCommit }: SliderProps) {
  return (
    <input
      type="range"
      className="cp-slider"
      aria-label={label}
      min={min}
      max={max}
      step={step}
      value={value}
      onChange={(e) => {
        onChange(Number(e.currentTarget.value));
      }}
      onPointerUp={(e) => onCommit?.(Number(e.currentTarget.value))}
      onKeyUp={(e) => {
        if (
          e.key.startsWith("Arrow") ||
          e.key === "Home" ||
          e.key === "End" ||
          e.key.startsWith("Page")
        ) {
          onCommit?.(Number(e.currentTarget.value));
        }
      }}
    />
  );
}

/** Número com commit em Enter/blur (sem comandos a cada tecla). */
export interface NumberFieldProps {
  value: number;
  label: string;
  min?: number;
  max?: number;
  step?: number;
  onCommit: (value: number) => void;
  disabled?: boolean;
}

export function NumberField({
  value,
  label,
  min,
  max,
  step = 1,
  onCommit,
  disabled,
}: NumberFieldProps) {
  const [draft, setDraftState] = useState<string | null>(null);
  // espelho síncrono do rascunho: `Enter` commita e depois dá `blur()` — sem o ref o blur veria o
  // rascunho antigo (estado ainda não atualizado) e commitaria de novo.
  const draftRef = useRef<string | null>(null);
  const setDraft = (v: string | null) => {
    draftRef.current = v;
    setDraftState(v);
  };
  const shown = draft ?? String(Number.isInteger(value) ? value : Number(value.toFixed(3)));
  const clamp = (n: number) => Math.min(max ?? Infinity, Math.max(min ?? -Infinity, n));
  const commit = () => {
    const d = draftRef.current;
    if (d === null) return;
    setDraft(null);
    const n = Number(d.replace(",", "."));
    if (!Number.isFinite(n)) return;
    const clamped = clamp(n);
    if (clamped !== value) onCommit(clamped);
  };
  return (
    <input
      className="cp-input"
      type="text"
      inputMode="decimal"
      aria-label={label}
      disabled={disabled}
      value={shown}
      data-step={step}
      onChange={(e) => {
        setDraft(e.currentTarget.value);
      }}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          commit();
          e.currentTarget.blur();
        } else if (e.key === "Escape") {
          setDraft(null);
          e.currentTarget.blur();
        } else if (e.key === "ArrowUp" || e.key === "ArrowDown") {
          e.preventDefault();
          const d = (e.key === "ArrowUp" ? 1 : -1) * step * (e.shiftKey ? 10 : 1);
          const n = clamp(value + d);
          setDraft(null);
          if (n !== value) onCommit(n);
        }
      }}
    />
  );
}

// ------------------------------------------------------------------------------------ abas

export interface TabItem {
  id: string;
  label: ReactNode;
  closable?: boolean;
  title?: string;
}

export interface TabsProps {
  items: TabItem[];
  active: string | null;
  onSelect: (id: string) => void;
  onClose?: (id: string) => void;
  onContextMenu?: (id: string, x: number, y: number) => void;
  label: string;
  trailing?: ReactNode;
  onDoubleClick?: (id: string) => void;
}

export function Tabs({
  items,
  active,
  onSelect,
  onClose,
  onContextMenu,
  onDoubleClick,
  label,
  trailing,
}: TabsProps) {
  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    const i = items.findIndex((t) => t.id === active);
    if (i < 0) return;
    const n = (i + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length;
    const next = items[n];
    if (next) {
      e.preventDefault();
      onSelect(next.id);
    }
  };
  return (
    <div className="cp-tabs" role="tablist" aria-label={label} onKeyDown={onKey}>
      {items.map((t) => (
        <div
          key={t.id}
          role="tab"
          tabIndex={t.id === active ? 0 : -1}
          aria-selected={t.id === active}
          title={t.title}
          className="cp-tab"
          data-tab-id={t.id}
          onClick={() => {
            onSelect(t.id);
          }}
          onDoubleClick={() => onDoubleClick?.(t.id)}
          onAuxClick={(e) => {
            if (e.button === 1 && t.closable) onClose?.(t.id);
          }}
          onContextMenu={(e) => {
            if (!onContextMenu) return;
            e.preventDefault();
            onContextMenu(t.id, e.clientX, e.clientY);
          }}
        >
          <span>{t.label}</span>
          {t.closable && onClose && (
            <button
              type="button"
              className="cp-tab-close"
              aria-label={`${typeof t.title === "string" ? t.title : t.id} ×`}
              tabIndex={-1}
              onClick={(e) => {
                e.stopPropagation();
                onClose(t.id);
              }}
            >
              <Icon name="close" size={12} />
            </button>
          )}
        </div>
      ))}
      {trailing}
    </div>
  );
}

// ------------------------------------------------------------------------------------ tooltip

export function Tooltip({ text, children }: { text: string; children: ReactNode }) {
  const [pos, setPos] = useState<{ x: number; y: number } | null>(null);
  const timer = useRef<number | undefined>(undefined);
  const show = (el: HTMLElement) => {
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      const r = el.getBoundingClientRect();
      setPos({ x: r.left + r.width / 2, y: r.bottom + 6 });
    }, 450);
  };
  const hide = () => {
    window.clearTimeout(timer.current);
    setPos(null);
  };
  useEffect(
    () => () => {
      window.clearTimeout(timer.current);
    },
    [],
  );
  return (
    <span
      style={{ display: "inline-flex" }}
      onPointerEnter={(e) => {
        show(e.currentTarget);
      }}
      onPointerLeave={hide}
      onFocus={(e) => {
        show(e.currentTarget);
      }}
      onBlur={hide}
      onPointerDown={hide}
    >
      {children}
      {pos &&
        createPortal(
          <div
            className="cp-tooltip"
            role="tooltip"
            style={{ left: pos.x, top: pos.y, transform: "translateX(-50%)" }}
          >
            {text}
          </div>,
          document.body,
        )}
    </span>
  );
}

// ------------------------------------------------------------------------------------ menu

export type MenuEntry =
  | {
      type: "item";
      id: string;
      label: string;
      shortcut?: string;
      disabled?: boolean;
      danger?: boolean;
    }
  | { type: "separator" };

export interface MenuProps {
  entries: MenuEntry[];
  x: number;
  y: number;
  label: string;
  onSelect: (id: string) => void;
  onClose: () => void;
}

/** Menu de contexto/popover com navegação por teclado (↑↓ Home End Enter Esc) e foco devolvido. */
export function Menu({ entries, x, y, label, onSelect, onClose }: MenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const restore = useRef<Element | null>(null);
  const [pos, setPos] = useState({ x, y });

  useLayoutEffect(() => {
    restore.current = document.activeElement;
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    setPos({
      x: Math.max(4, Math.min(x, window.innerWidth - r.width - 4)),
      y: Math.max(4, Math.min(y, window.innerHeight - r.height - 4)),
    });
    el.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    return () => {
      if (restore.current instanceof HTMLElement) restore.current.focus();
    };
  }, [x, y]);

  useEffect(() => {
    const away = (e: PointerEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) onClose();
    };
    window.addEventListener("pointerdown", away, true);
    return () => {
      window.removeEventListener("pointerdown", away, true);
    };
  }, [onClose]);

  const onKey = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const btns = [
      ...(ref.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []),
    ];
    const i = btns.indexOf(document.activeElement as HTMLButtonElement);
    const go = (n: number) => {
      e.preventDefault();
      btns[(n + btns.length) % btns.length]?.focus();
    };
    if (e.key === "ArrowDown") go(i + 1);
    else if (e.key === "ArrowUp") go(i - 1);
    else if (e.key === "Home") go(0);
    else if (e.key === "End") go(btns.length - 1);
    else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "Tab") {
      e.preventDefault();
      onClose();
    }
  };

  return createPortal(
    <div
      ref={ref}
      className="cp-menu"
      role="menu"
      aria-label={label}
      style={{ left: pos.x, top: pos.y }}
      onKeyDown={onKey}
    >
      {entries.map((en, i) =>
        en.type === "separator" ? (
          <div key={`sep-${String(i)}`} className="cp-menu-sep" role="separator" />
        ) : (
          <button
            key={en.id}
            type="button"
            role="menuitem"
            className="cp-menu-item"
            disabled={en.disabled}
            data-danger={en.danger ? "true" : undefined}
            data-menu-id={en.id}
            onClick={() => {
              onSelect(en.id);
              onClose();
            }}
          >
            <span>{en.label}</span>
            {en.shortcut && <span className="cp-kbd">{en.shortcut}</span>}
          </button>
        ),
      )}
    </div>,
    document.body,
  );
}

// ------------------------------------------------------------------------------------ diálogo

export interface DialogProps {
  title: string;
  open: boolean;
  onClose: () => void;
  children: ReactNode;
  footer?: ReactNode;
  /** Foco inicial: seletor CSS dentro do diálogo (padrão: primeiro controle). */
  initialFocus?: string;
}

const FOCUSABLE =
  'button:not(:disabled), [href], input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex]:not([tabindex="-1"])';

/** Diálogo modal: Esc fecha, Tab circula dentro dele, o foco volta a quem o abriu. */
export function Dialog({ title, open, onClose, children, footer, initialFocus }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const opener = useRef<Element | null>(null);

  useLayoutEffect(() => {
    if (!open) return;
    opener.current = document.activeElement;
    const el = ref.current;
    const target =
      (initialFocus ? el?.querySelector<HTMLElement>(initialFocus) : null) ??
      el?.querySelector<HTMLElement>(FOCUSABLE);
    target?.focus();
    return () => {
      if (opener.current instanceof HTMLElement) opener.current.focus();
    };
  }, [open, initialFocus]);

  const onKey = useCallback(
    (e: ReactKeyboardEvent<HTMLDivElement>) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
        return;
      }
      if (e.key !== "Tab") return;
      const els = [...(ref.current?.querySelectorAll<HTMLElement>(FOCUSABLE) ?? [])];
      const first = els[0];
      const last = els[els.length - 1];
      if (!first || !last) return;
      if (e.shiftKey && document.activeElement === first) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && document.activeElement === last) {
        e.preventDefault();
        first.focus();
      }
    },
    [onClose],
  );

  if (!open) return null;
  return createPortal(
    <div
      className="cp-backdrop"
      onPointerDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        ref={ref}
        className="cp-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        onKeyDown={onKey}
      >
        <div className="cp-dialog-head" id={titleId}>
          {title}
        </div>
        <div className="cp-dialog-body">{children}</div>
        {footer && <div className="cp-dialog-foot">{footer}</div>}
      </div>
    </div>,
    document.body,
  );
}

// ------------------------------------------------------------------------------------ toasts

export interface ToastItem {
  id: number;
  tone: "info" | "success" | "error";
  title: string;
  detail?: string | undefined;
}

export function Toasts({
  items,
  onDismiss,
}: {
  items: ToastItem[];
  onDismiss: (id: number) => void;
}) {
  return (
    <div className="cp-toasts" role="region" aria-live="polite" aria-label="notifications">
      {items.map((t) => (
        <div
          key={t.id}
          className="cp-toast"
          data-tone={t.tone}
          role={t.tone === "error" ? "alert" : "status"}
        >
          <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
            <span className="cp-toast-title">{t.title}</span>
            <button
              type="button"
              className="cp-tab-close"
              aria-label="×"
              onClick={() => {
                onDismiss(t.id);
              }}
            >
              <Icon name="close" size={12} />
            </button>
          </div>
          {t.detail && <div className="cp-toast-detail">{t.detail}</div>}
        </div>
      ))}
    </div>
  );
}

// ------------------------------------------------------------------------------------ estados

export function EmptyState({
  icon,
  title,
  hint,
  action,
}: {
  icon?: IconName;
  title: string;
  hint?: string;
  action?: ReactNode;
}) {
  return (
    <div className="cp-empty">
      {icon && <Icon name={icon} size={28} />}
      <div style={{ color: "var(--text)", fontWeight: 600 }}>{title}</div>
      {hint && <div>{hint}</div>}
      {action}
    </div>
  );
}

export function Spinner({ label }: { label: string }) {
  return <span className="cp-spinner" role="status" aria-label={label} />;
}

export function Badge({
  children,
  tone,
}: {
  children: ReactNode;
  tone?: "danger" | "warning" | "success";
}) {
  return (
    <span className="cp-badge" data-tone={tone}>
      {children}
    </span>
  );
}

// ------------------------------------------------------------------------------- splitter

export interface SplitterProps {
  /** `col` = divisor vertical (redimensiona largura); `row` = horizontal (altura). */
  dir: "col" | "row";
  label: string;
  /** Tamanho atual (px) do painel controlado. */
  size: number;
  min: number;
  max: number;
  /** `-1` quando o painel controlado fica depois do divisor (arrastar para a esquerda/cima cresce). */
  sign?: 1 | -1;
  onResize: (size: number) => void;
  onCommit?: (size: number) => void;
}

/** Divisor com mouse (pointer capture) e teclado (setas ±16 px, Shift ±64 px). */
export function Splitter({
  dir,
  label,
  size,
  min,
  max,
  sign = 1,
  onResize,
  onCommit,
}: SplitterProps) {
  const drag = useRef<{ start: number; size: number } | null>(null);
  const [active, setActive] = useState(false);
  const clamp = (v: number) => Math.min(max, Math.max(min, v));
  const down = (e: ReactPointerEvent<HTMLDivElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    drag.current = { start: dir === "col" ? e.clientX : e.clientY, size };
    setActive(true);
  };
  const move = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    if (!d) return;
    const delta = ((dir === "col" ? e.clientX : e.clientY) - d.start) * sign;
    onResize(clamp(d.size + delta));
  };
  const up = (e: ReactPointerEvent<HTMLDivElement>) => {
    const d = drag.current;
    drag.current = null;
    setActive(false);
    if (!d) return;
    const delta = ((dir === "col" ? e.clientX : e.clientY) - d.start) * sign;
    onCommit?.(clamp(d.size + delta));
  };
  const key = (e: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 64 : 16;
    const grow =
      dir === "col"
        ? sign === 1
          ? "ArrowRight"
          : "ArrowLeft"
        : sign === 1
          ? "ArrowDown"
          : "ArrowUp";
    const shrink =
      dir === "col"
        ? sign === 1
          ? "ArrowLeft"
          : "ArrowRight"
        : sign === 1
          ? "ArrowUp"
          : "ArrowDown";
    if (e.key !== grow && e.key !== shrink) return;
    e.preventDefault();
    const n = clamp(size + (e.key === grow ? step : -step));
    onResize(n);
    onCommit?.(n);
  };
  return (
    <div
      className="cp-splitter"
      data-dir={dir}
      data-active={active}
      role="separator"
      aria-orientation={dir === "col" ? "vertical" : "horizontal"}
      aria-label={label}
      aria-valuenow={Math.round(size)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={down}
      onPointerMove={move}
      onPointerUp={up}
      onKeyDown={key}
    />
  );
}
