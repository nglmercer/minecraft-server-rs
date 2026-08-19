import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Backups } from "../components/Backups";
import { Console } from "../components/Console";
import { Files } from "../components/Files";
import { Mods } from "../components/Mods";
import { Banner, Button, Card, Field, Input, Select, StatusPill, formatUptime } from "../components/ui";
import type { Server, Status, User } from "../types";

type Tab = "console" | "files" | "plugins" | "backups" | "settings";

export function ServerDetail({
  id,
  user,
  onBack,
}: {
  id: string;
  user: User;
  onBack: () => void;
}) {
  const [server, setServer] = useState<Server | null>(null);
  const [tab, setTab] = useState<Tab>("console");
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      setServer(await api.server(id));
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not load the server");
    }
  }

  useEffect(() => {
    void refresh();
  }, [id]);

  // The console socket already streams status; poll only for pid and uptime.
  useEffect(() => {
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, [id]);

  function onStatus(status: Status) {
    setServer((prev) => (prev ? { ...prev, status } : prev));
  }

  async function power(action: "start" | "stop" | "restart" | "kill") {
    setError(null);
    try {
      setServer(await api.power(id, action));
    } catch (e) {
      setError(e instanceof Error ? e.message : "action failed");
    }
  }

  if (!server) {
    return (
      <div class="mx-auto max-w-6xl px-6 py-8">
        {error ? <Banner kind="error">{error}</Banner> : <p class="text-fg-muted">Loading…</p>}
      </div>
    );
  }

  const running = server.status !== "offline" && server.status !== "crashed";

  return (
    <div class="mx-auto flex h-full w-full max-w-6xl flex-col gap-5 px-6 py-6">
      <header class="flex flex-wrap items-center justify-between gap-4">
        <div class="flex items-center gap-3">
          <Button variant="ghost" onClick={onBack}>
            ← Servers
          </Button>
          <div>
            <h1 class="text-xl font-semibold">{server.name}</h1>
            <p class="text-xs text-fg-muted">
              {server.core} {server.version} · port {server.port} · pid {server.pid ?? "—"} · up{" "}
              {formatUptime(server.uptime_secs)}
              {server.metrics && (
                <>
                  {" · "}
                  <span class="tabular-nums">
                    {server.metrics.cpu_percent.toFixed(0)}% CPU ·{" "}
                    {server.metrics.memory_mb} MB
                  </span>
                </>
              )}
            </p>
          </div>
        </div>

        <div class="flex items-center gap-3">
          <StatusPill status={server.status} />
          <Button variant="primary" disabled={running} onClick={() => power("start")}>
            Start
          </Button>
          <Button disabled={!running} onClick={() => power("restart")}>
            Restart
          </Button>
          <Button variant="danger" disabled={!running} onClick={() => power("stop")}>
            Stop
          </Button>
          <Button variant="ghost" disabled={!running} onClick={() => power("kill")}>
            Kill
          </Button>
        </div>
      </header>

      {error && <Banner kind="error">{error}</Banner>}

      {!server.eula_accepted && (
        <Banner kind="info">
          The Minecraft EULA has not been accepted for this server, so it will refuse to start.
          Accept it under <strong>Settings</strong>.
        </Banner>
      )}

      <nav class="flex gap-1 border-b border-ink-700">
        {(["console", "files", "plugins", "backups", "settings"] as Tab[]).map((name) => (
          <button
            key={name}
            onClick={() => setTab(name)}
            class={`-mb-px border-b-2 px-4 py-2 text-sm font-medium capitalize transition-colors ${
              tab === name
                ? "border-accent text-fg"
                : "border-transparent text-fg-muted hover:text-fg"
            }`}
          >
            {name}
          </button>
        ))}
      </nav>

      <div class="flex min-h-0 flex-1 flex-col">
        {tab === "console" && <Console serverId={id} onStatus={onStatus} />}
        {tab === "files" && <Files serverId={id} />}
        {tab === "plugins" && <Mods server={server} />}
        {tab === "backups" && <Backups serverId={id} status={server.status} />}
        {tab === "settings" && (
          <Settings server={server} user={user} onSaved={refresh} onDeleted={onBack} />
        )}
      </div>
    </div>
  );
}

function Settings({
  server,
  user,
  onSaved,
  onDeleted,
}: {
  server: Server;
  user: User;
  onSaved: () => void;
  onDeleted: () => void;
}) {
  const [form, setForm] = useState({
    name: server.name,
    port: server.port,
    java_major: server.java_major,
    min_mb: server.memory.min_mb,
    max_mb: server.memory.max_mb,
    jvm_args: server.jvm_args.join(" "),
    eula_accepted: server.eula_accepted,
    auto_restart: server.policy.auto_restart,
    max_retries: server.policy.max_retries,
    retry_delay_secs: server.policy.retry_delay_secs,
    stop_timeout_secs: server.policy.stop_timeout_secs,
  });
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const set = (patch: Partial<typeof form>) => {
    setForm((f) => ({ ...f, ...patch }));
    setSaved(false);
  };

  async function submit(event: Event) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.updateServer(server.id, {
        name: form.name,
        port: form.port,
        java_major: form.java_major,
        memory: { min_mb: form.min_mb, max_mb: form.max_mb },
        jvm_args: form.jvm_args.split(/\s+/).filter(Boolean),
        eula_accepted: form.eula_accepted,
        policy: {
          ...server.policy,
          auto_restart: form.auto_restart,
          max_retries: form.max_retries,
          retry_delay_secs: form.retry_delay_secs,
          stop_timeout_secs: form.stop_timeout_secs,
        },
      });
      setSaved(true);
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not save");
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!confirm(`Remove "${server.name}" from the panel? Its files stay on disk.`)) return;
    await api.deleteServer(server.id);
    onDeleted();
  }

  return (
    <form onSubmit={submit} class="space-y-5 overflow-y-auto pb-6">
      {error && <Banner kind="error">{error}</Banner>}
      {saved && <Banner kind="info">Saved. Changes apply the next time the server starts.</Banner>}

      <Card title="Server">
        <div class="grid gap-4 sm:grid-cols-2">
          <Field label="Name">
            <Input value={form.name} onInput={(e) => set({ name: (e.target as HTMLInputElement).value })} />
          </Field>
          <Field label="Port">
            <Input
              type="number"
              value={form.port}
              onInput={(e) => set({ port: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
          <Field label="Java version">
            <Select
              value={String(form.java_major)}
              onChange={(e) => set({ java_major: Number((e.target as HTMLSelectElement).value) })}
            >
              {[8, 11, 16, 17, 21, 25].map((v) => (
                <option key={v} value={v}>
                  Java {v}
                </option>
              ))}
            </Select>
          </Field>
          <div class="grid grid-cols-2 gap-3">
            <Field label="Min RAM (MB)">
              <Input
                type="number"
                value={form.min_mb}
                onInput={(e) => set({ min_mb: Number((e.target as HTMLInputElement).value) })}
              />
            </Field>
            <Field label="Max RAM (MB)">
              <Input
                type="number"
                value={form.max_mb}
                onInput={(e) => set({ max_mb: Number((e.target as HTMLInputElement).value) })}
              />
            </Field>
          </div>
          <div class="sm:col-span-2">
            <Field label="Extra JVM flags" hint="Space separated, inserted before -jar.">
              <Input
                value={form.jvm_args}
                placeholder="-XX:+UseG1GC -XX:MaxGCPauseMillis=200"
                onInput={(e) => set({ jvm_args: (e.target as HTMLInputElement).value })}
              />
            </Field>
          </div>
        </div>

        <label class="mt-4 flex items-center gap-2.5 text-sm text-fg-muted">
          <input
            type="checkbox"
            checked={form.eula_accepted}
            onChange={(e) => set({ eula_accepted: (e.target as HTMLInputElement).checked })}
            class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          Minecraft EULA accepted
        </label>
      </Card>

      <Card title="Crash recovery">
        <div class="grid gap-4 sm:grid-cols-3">
          <Field label="Max restart attempts">
            <Input
              type="number"
              value={form.max_retries}
              onInput={(e) => set({ max_retries: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
          <Field label="Retry delay (s)">
            <Input
              type="number"
              value={form.retry_delay_secs}
              onInput={(e) => set({ retry_delay_secs: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
          <Field label="Graceful stop timeout (s)">
            <Input
              type="number"
              value={form.stop_timeout_secs}
              onInput={(e) => set({ stop_timeout_secs: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
        </div>
        <label class="mt-4 flex items-center gap-2.5 text-sm text-fg-muted">
          <input
            type="checkbox"
            checked={form.auto_restart}
            onChange={(e) => set({ auto_restart: (e.target as HTMLInputElement).checked })}
            class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          Restart automatically after a crash
        </label>
      </Card>

      <div class="flex items-center gap-3">
        <Button type="submit" variant="primary" disabled={busy}>
          {busy ? "Saving…" : "Save changes"}
        </Button>
        {user.admin && (
          <Button type="button" variant="danger" onClick={remove}>
            Remove server
          </Button>
        )}
      </div>
    </form>
  );
}
