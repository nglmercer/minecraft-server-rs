import type { ComponentChildren } from "preact";
import { createContext } from "preact";
import { useCallback, useContext, useMemo, useState } from "preact/hooks";

/** How long a toast stays before it fades out. */
const LIFETIME_MS = 5000;

type Tone = "success" | "error" | "info";

interface Toast {
  id: number;
  tone: Tone;
  message: string;
}

interface Toaster {
  /** Show a transient message. Errors stay a little longer. */
  show: (tone: Tone, message: string) => void;
  success: (message: string) => void;
  error: (message: string) => void;
  info: (message: string) => void;
}

const ToastContext = createContext<Toaster | null>(null);

/**
 * Transient feedback, replacing the inline banners that used to push layout
 * around every time an action succeeded.
 */
export function ToastProvider({ children }: { children: ComponentChildren }) {
  const [toasts, setToasts] = useState<Toast[]>([]);

  const show = useCallback((tone: Tone, message: string) => {
    const id = Date.now() + Math.random();
    setToasts((prev) => [...prev, { id, tone, message }]);

    // Errors get longer, because they are the ones worth reading twice.
    const lifetime = tone === "error" ? LIFETIME_MS * 1.6 : LIFETIME_MS;
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), lifetime);
  }, []);

  const value = useMemo<Toaster>(
    () => ({
      show,
      success: (message) => show("success", message),
      error: (message) => show("error", message),
      info: (message) => show("info", message),
    }),
    [show],
  );

  const tones: Record<Tone, string> = {
    success: "border-accent/40 bg-accent/10 text-accent",
    error: "border-red-500/40 bg-red-500/10 text-red-200",
    info: "border-sky-500/40 bg-sky-500/10 text-sky-100",
  };

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div
        class="pointer-events-none fixed bottom-4 right-4 z-[60] flex w-full max-w-sm flex-col gap-2"
        role="status"
        aria-live="polite"
      >
        {toasts.map((toast) => (
          <div
            key={toast.id}
            class={`pointer-events-auto animate-[fade-in_150ms_ease-out] rounded-xl border px-4 py-3 text-sm shadow-lg backdrop-blur ${tones[toast.tone]}`}
            onClick={() => setToasts((prev) => prev.filter((t) => t.id !== toast.id))}
          >
            {toast.message}
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

/** Show transient success/error/info messages. */
export function useToast(): Toaster {
  const context = useContext(ToastContext);
  if (!context) throw new Error("useToast must be used inside a ToastProvider");
  return context;
}
