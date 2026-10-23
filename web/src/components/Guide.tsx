import { useEffect, useState } from "preact/hooks";
import { useT } from "../i18n";
import { Modal } from "./Modal";
import { Button } from "./ui";

export interface GuideStep {
  title: string;
  body: string;
  enter?: () => void;
}

/** Bundled, keyboard-accessible tour; works offline without CDN assets. */
export function Guide({
  steps,
  storageKey,
}: {
  steps: GuideStep[];
  storageKey?: string;
}) {
  const t = useT();
  const [step, setStep] = useState<number | null>(() => {
    if (!storageKey) return null;
    try {
      return localStorage.getItem(`mcpanel:guide:v1:${storageKey}`) ? null : 0;
    } catch {
      return null;
    }
  });
  const current = step === null ? null : steps[step];
  useEffect(() => {
    current?.enter?.();
  }, [step]);
  function close() {
    setStep(null);
    if (storageKey) {
      try {
        localStorage.setItem(`mcpanel:guide:v1:${storageKey}`, "seen");
      } catch {
        /* optional */
      }
    }
  }
  return (
    <div class="shrink-0">
      <Button variant="ghost" onClick={() => setStep(0)}>
        {t("guide.help")}
      </Button>
      {current && (
        <Modal
          title={current.title}
          onClose={close}
          footer={
            <>
              <Button variant="ghost" onClick={close}>
                {t("guide.skip")}
              </Button>
              <Button
                disabled={step === 0}
                onClick={() => setStep((step ?? 0) - 1)}
              >
                {t("guide.previous")}
              </Button>
              <Button
                variant="primary"
                onClick={() =>
                  (step ?? 0) + 1 < steps.length
                    ? setStep((step ?? 0) + 1)
                    : close()
                }
              >
                {(step ?? 0) + 1 < steps.length
                  ? t("guide.next")
                  : t("guide.done")}
              </Button>
            </>
          }
        >
          <p class="mb-3 text-xs text-fg-muted" aria-live="polite">
            {t("guide.step", { current: (step ?? 0) + 1, total: steps.length })}
          </p>
          <p class="text-sm leading-relaxed">{current.body}</p>
        </Modal>
      )}
    </div>
  );
}
