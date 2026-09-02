import { useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Field, Input } from "../components/ui";
import { useT } from "../i18n";

function tokenFromFragment(): string {
  // Token is in URL fragment: /recovery#<token> or #/recovery#<token>
  const hash = location.hash || "";
  // Handle both "#<token>" and "#/recovery#<token>" etc: take after last '#'
  const last = hash.lastIndexOf("#");
  if (last !== -1 && last < hash.length - 1) {
    const candidate = hash.slice(last + 1);
    // If hash is like "#/recovery" the candidate is "/recovery" not token; check if it looks like hex
    if (/^[a-f0-9]{64}$/.test(candidate)) return candidate;
    // Also support token directly after "/recovery#"
    if (candidate.includes("#")) {
      const inner = candidate.slice(candidate.lastIndexOf("#") + 1);
      if (/^[a-f0-9]{64}$/.test(inner)) return inner;
    }
  }
  // Also check pathname fragment via location.hash splitting? For path-based /recovery#token, location.hash is "#token"
  if (/^#[a-f0-9]{64}$/.test(hash)) return hash.slice(1);
  // fallback: try parsing full href fragment
  const hrefHash = location.href.split("#").pop() || "";
  if (/^[a-f0-9]{64}$/.test(hrefHash)) return hrefHash;
  return "";
}

export function Recovery({ onDone }: { onDone: () => void }) {
  const t = useT();
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [success, setSuccess] = useState(false);
  const token = tokenFromFragment();

  async function submit(e: Event) {
    e.preventDefault();
    if (!token) {
      setError(t("recovery.missingToken"));
      return;
    }
    if (password !== confirm) {
      setError(t("setup.mismatch"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.recoveryReset({ token, password, confirm });
      setSuccess(true);
    } catch (err) {
      setError(err instanceof Error ? err.message : t("recovery.failed"));
    } finally {
      setBusy(false);
    }
  }

  if (success) {
    return (
      <div class="grid min-h-full place-items-center px-6 py-16">
        <div class="w-full max-w-sm space-y-4 rounded-2xl border border-ink-700 bg-ink-850 p-8 text-center">
          <p class="text-sm text-green-400">{t("recovery.success")}</p>
          <Button variant="primary" class="w-full" onClick={onDone}>
            {t("recovery.goLogin")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div class="grid min-h-full place-items-center px-6 py-16">
      <form onSubmit={submit} class="w-full max-w-sm space-y-5 rounded-2xl border border-ink-700 bg-ink-850 p-8">
        <div class="space-y-1">
          <h1 class="text-lg font-semibold">{t("recovery.heading")}</h1>
          <p class="text-sm text-fg-muted">{t("recovery.subtitle")}</p>
        </div>
        {error && <Banner kind="error">{error}</Banner>}
        {!token && <Banner kind="error">{t("recovery.missingToken")}</Banner>}
        <Field label={t("recovery.newPassword")}>
          <Input type="password" value={password} autocomplete="new-password" onInput={(e) => setPassword((e.target as HTMLInputElement).value)} />
        </Field>
        <Field label={t("setup.confirm")}>
          <Input type="password" value={confirm} autocomplete="new-password" onInput={(e) => setConfirm((e.target as HTMLInputElement).value)} />
        </Field>
        <Button type="submit" variant="primary" class="w-full" disabled={busy || !token}>
          {busy ? t("common.saving") : t("recovery.reset")}
        </Button>
      </form>
    </div>
  );
}
