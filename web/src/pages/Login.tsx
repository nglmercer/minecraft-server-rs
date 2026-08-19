import { useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Field, Input } from "../components/ui";
import type { User } from "../types";

export function Login({ onSignedIn }: { onSignedIn: (user: User) => void }) {
  const [username, setUsername] = useState("admin");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(event: Event) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      onSignedIn(await api.login(username, password));
    } catch (e) {
      setError(e instanceof Error ? e.message : "login failed");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div class="grid min-h-full place-items-center px-6 py-16">
      <form
        onSubmit={submit}
        class="w-full max-w-sm space-y-5 rounded-2xl border border-ink-700 bg-ink-850 p-8"
      >
        <div class="space-y-1">
          <div class="flex items-center gap-2.5">
            <span class="grid size-8 place-items-center rounded-lg bg-accent text-ink-950 font-bold">
              M
            </span>
            <h1 class="text-lg font-semibold">Minecraft Panel</h1>
          </div>
          <p class="text-sm text-fg-muted">Sign in to manage your servers.</p>
        </div>

        {error && <Banner kind="error">{error}</Banner>}

        <Field label="Username">
          <Input
            value={username}
            autocomplete="username"
            onInput={(e) => setUsername((e.target as HTMLInputElement).value)}
          />
        </Field>

        <Field label="Password">
          <Input
            type="password"
            value={password}
            autocomplete="current-password"
            onInput={(e) => setPassword((e.target as HTMLInputElement).value)}
          />
        </Field>

        <Button type="submit" variant="primary" class="w-full" disabled={busy}>
          {busy ? "Signing in…" : "Sign in"}
        </Button>

        <p class="text-center text-xs text-fg-muted">
          First run? The initial password was printed in the panel's console.
        </p>
      </form>
    </div>
  );
}
