import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Field, Input } from "../components/ui";
import { useT } from "../i18n";

export function Setup({ onDone }: { onDone: () => void }) {
  const t = useT();
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [needsSetup, setNeedsSetup] = useState<boolean | null>(null);

  useEffect(() => {
    api
      .setupStatus()
      .then((r) => setNeedsSetup(r.needs_setup))
      .catch(() => setNeedsSetup(false));
  }, []);

  async function submit(e: Event) {
    e.preventDefault();
    if (password !== confirm) {
      setError(t("setup.mismatch"));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await api.setup({ username, password, confirm });
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : t("setup.failed"));
    } finally {
      setBusy(false);
    }
  }

  if (needsSetup === null) {
    return <div class="grid h-full place-items-center text-fg-muted">{t("common.loading")}</div>;
  }
  if (!needsSetup) {
    return (
      <div class="grid min-h-full place-items-center px-6 py-16">
        <div class="w-full max-w-sm rounded-2xl border border-ink-700 bg-ink-850 p-8 text-center">
          <p class="text-sm">{t("setup.alreadyDone")}</p>
          <a href="/" class="mt-4 inline-block text-accent underline">
            {t("setup.goLogin")}
          </a>
        </div>
      </div>
    );
  }

  return (
    <div class="grid min-h-full place-items-center px-6 py-16">
      <form onSubmit={submit} class="w-full max-w-sm space-y-5 rounded-2xl border border-ink-700 bg-ink-850 p-8">
        <div class="space-y-1">
          <h1 class="text-lg font-semibold">{t("setup.heading")}</h1>
          <p class="text-sm text-fg-muted">{t("setup.subtitle")}</p>
        </div>
        {error && <Banner kind="error">{error}</Banner>}
        <Field label={t("login.username")}>
          <Input value={username} autocomplete="username" onInput={(e) => setUsername((e.target as HTMLInputElement).value)} />
        </Field>
        <Field label={t("login.password")}>
          <Input type="password" value={password} autocomplete="new-password" onInput={(e) => setPassword((e.target as HTMLInputElement).value)} />
        </Field>
        <Field label={t("setup.confirm")}>
          <Input type="password" value={confirm} autocomplete="new-password" onInput={(e) => setConfirm((e.target as HTMLInputElement).value)} />
        </Field>
        <Button type="submit" variant="primary" class="w-full" disabled={busy}>
          {busy ? t("common.creating") : t("setup.create")}
        </Button>
      </form>
    </div>
  );
}
