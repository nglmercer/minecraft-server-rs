import { useEffect, useLayoutEffect, useState } from "preact/hooks";
import { api } from "../api";
import type { CompatibilityView } from "../types";
import { useT } from "../i18n";
import { Banner, Button } from "./ui";

export function Compatibility({
  core,
  version,
  javaMajor,
  onAllowedChange,
}: {
  core: string;
  version: string;
  javaMajor: number;
  onAllowedChange?: (allowed: boolean) => void;
}) {
  const t = useT();
  const [snapshot, setSnapshot] = useState<{
    key: string;
    value: CompatibilityView;
  } | null>(null);
  const [attempt, setAttempt] = useState(0);
  const [acknowledgedKey, setAcknowledgedKey] = useState<string | null>(null);
  const requestKey = `${core}:${version}:${attempt}`;
  const result = snapshot?.key === requestKey ? snapshot.value : null;
  const loading = Boolean(version) && result === null;
  const acknowledged = acknowledgedKey === requestKey;
  useEffect(() => {
    let active = true;
    if (!version) return;
    api
      .compatibility(core, version)
      .then((value) => {
        if (active) setSnapshot({ key: requestKey, value });
      })
      .catch(() => {
        if (active)
          setSnapshot({
            key: requestKey,
            value: {
              availability: "unknown",
              support: "unknown",
              java_minimum: null,
            },
          });
      });
    return () => {
      active = false;
    };
  }, [core, version, attempt]);
  const warning =
    result?.support === "unsupported" || result?.support === "deprecated";
  const needsReview = warning || result?.availability === "unknown";
  useLayoutEffect(() => {
    onAllowedChange?.(
      Boolean(version) &&
        !loading &&
        Boolean(result) &&
        result?.availability !== "unavailable" &&
        (!result?.java_minimum || javaMajor >= result.java_minimum) &&
        (!needsReview || acknowledged),
    );
  }, [
    version,
    loading,
    result,
    javaMajor,
    needsReview,
    acknowledged,
    onAllowedChange,
  ]);
  if (!version) return null;
  return (
    <section
      aria-label={t("compatibility.title")}
      class="space-y-2 text-sm"
    >
      <h3 class="text-xs font-medium text-fg-muted">
        {t("compatibility.title")} · {core} {version}
      </h3>
      <div class="flex flex-wrap gap-x-4 gap-y-1 text-xs text-fg-muted" aria-live="polite">
        {loading ? (
          t("common.loading")
        ) : (
          <>
            <p>{t(`compatibility.${result?.availability ?? "unknown"}`)}</p>
            <p>{t(`compatibility.support.${result?.support ?? "unknown"}`)}</p>
            {result?.java_minimum && (
              <p>{t("compatibility.java", { version: result.java_minimum })}</p>
            )}
          </>
        )}
      </div>
      {result?.java_minimum && javaMajor < result.java_minimum && (
        <Banner kind="error">
          {t("compatibility.javaMismatch", {
            selected: javaMajor,
            required: result.java_minimum,
          })}
        </Banner>
      )}
      {needsReview && (
        <Banner kind="info">
          <label class="flex items-start gap-2">
            <input
              type="checkbox"
              checked={acknowledged}
              onChange={(event) =>
                setAcknowledgedKey(
                  event.currentTarget.checked ? requestKey : null,
                )
              }
            />
            {t(warning ? "compatibility.acknowledge" : "compatibility.acknowledgeUnknown")}
          </label>
          <details class="mt-2 text-xs">
            <summary class="cursor-pointer text-fg-muted">{t("compatibility.details")}</summary>
            <p class="mt-2">{t(warning ? "compatibility.warning" : "compatibility.unverified")}</p>
          </details>
        </Banner>
      )}
      {result?.availability === "unavailable" && (
        <Banner kind="error">{t("compatibility.noBuild")}</Banner>
      )}
      <div class="flex flex-wrap items-center gap-3 text-xs">
        <Button
          type="button"
          variant="ghost"
          disabled={loading}
          onClick={() => setAttempt((n) => n + 1)}
        >
          {t("compatibility.retry")}
        </Button>
        {["paper", "folia", "velocity", "waterfall"].includes(core) && (
          <a
            class="text-accent underline"
            href={`https://papermc.io/downloads/${core}`}
            target="_blank"
            rel="noopener noreferrer"
          >
            {t("compatibility.official")}
          </a>
        )}
      </div>
    </section>
  );
}
