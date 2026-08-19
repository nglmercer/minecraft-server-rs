import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Card, Field, Input, Select, StatusPill, formatUptime } from "../components/ui";
import type { Server, SystemStats, User } from "../types";

export function Dashboard({
  user,
  onOpen,
}: {
  user: User;
  onOpen: (id: string) => void;
}) {
  const [servers, setServers] = useState<Server[]>([]);
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function refresh() {
    try {
      const [list, system] = await Promise.all([api.servers(), api.system()]);
      setServers(list);
      setStats(system);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not load servers");
    }
  }

  useEffect(() => {
    void refresh();
    // Polling keeps the list fresh without holding a socket open per server.
    const timer = setInterval(refresh, 4000);
    return () => clearInterval(timer);
  }, []);

  async function power(id: string, action: "start" | "stop" | "restart") {
    try {
      await api.power(id, action);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "action failed");
    }
  }

  return (
    <div class="mx-auto w-full max-w-6xl space-y-6 px-6 py-8">
      <header class="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 class="text-2xl font-semibold">Servers</h1>
          <p class="text-sm text-fg-muted">
            {servers.length} configured · {stats?.servers_online ?? 0} online
          </p>
        </div>
        {user.admin && (
          <Button variant="primary" onClick={() => setCreating((v) => !v)}>
            {creating ? "Cancel" : "New server"}
          </Button>
        )}
      </header>

      {stats && (
        <div class="grid gap-4 sm:grid-cols-3">
          <Stat label="Host CPU" value={`${stats.cpu_percent.toFixed(0)}%`} />
          <Stat
            label="Host memory"
            value={`${(stats.memory_used_mb / 1024).toFixed(1)} / ${(stats.memory_total_mb / 1024).toFixed(1)} GiB`}
          />
          <Stat label="Servers online" value={String(stats.servers_online)} />
        </div>
      )}

      {error && <Banner kind="error">{error}</Banner>}

      {creating && (
        <CreateServer
          onCreated={async () => {
            setCreating(false);
            await refresh();
          }}
        />
      )}

      <div class="grid gap-4">
        {servers.map((server) => (
          <article
            key={server.id}
            class="flex flex-wrap items-center justify-between gap-4 rounded-xl border border-ink-700 bg-ink-850 px-5 py-4"
          >
            <div class="min-w-0 space-y-1">
              <button
                class="truncate text-base font-semibold hover:text-accent"
                onClick={() => onOpen(server.id)}
              >
                {server.name}
              </button>
              <p class="text-xs text-fg-muted">
                {server.core} {server.version} · port {server.port} · Java {server.java_major} ·{" "}
                {server.memory.max_mb} MB · up {formatUptime(server.uptime_secs)}
                {server.metrics && (
                  <>
                    {" · "}
                    <span class="tabular-nums text-fg">
                      {server.metrics.cpu_percent.toFixed(0)}% CPU ·{" "}
                      {server.metrics.memory_mb} MB used
                    </span>
                  </>
                )}
              </p>
            </div>

            <div class="flex items-center gap-3">
              <StatusPill status={server.status} />
              <div class="flex gap-2">
                <Button
                  variant="primary"
                  disabled={server.status !== "offline" && server.status !== "crashed"}
                  onClick={() => power(server.id, "start")}
                >
                  Start
                </Button>
                <Button
                  disabled={server.status === "offline" || server.status === "crashed"}
                  onClick={() => power(server.id, "restart")}
                >
                  Restart
                </Button>
                <Button
                  variant="danger"
                  disabled={server.status === "offline" || server.status === "crashed"}
                  onClick={() => power(server.id, "stop")}
                >
                  Stop
                </Button>
                <Button variant="ghost" onClick={() => onOpen(server.id)}>
                  Manage
                </Button>
              </div>
            </div>
          </article>
        ))}

        {servers.length === 0 && !creating && (
          <Card>
            <p class="text-center text-sm text-fg-muted">
              No servers yet.{" "}
              {user.admin
                ? "Create one to get started — Java and the server jar are downloaded for you."
                : "Ask an administrator to grant you access to one."}
            </p>
          </Card>
        )}
      </div>
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div class="rounded-xl border border-ink-700 bg-ink-850 px-5 py-4">
      <p class="text-xs uppercase tracking-wider text-fg-muted">{label}</p>
      <p class="mt-1 text-xl font-semibold tabular-nums">{value}</p>
    </div>
  );
}

function CreateServer({ onCreated }: { onCreated: () => void }) {
  const [providers, setProviders] = useState<{ id: string; server: boolean }[]>([]);
  const [versions, setVersions] = useState<string[]>([]);
  const [form, setForm] = useState({
    name: "",
    core: "paper",
    version: "",
    java_major: 21,
    port: 25565,
    min_mb: 1024,
    max_mb: 4096,
    eula_accepted: false,
  });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.providers().then(setProviders).catch(() => {});
  }, []);

  useEffect(() => {
    setVersions([]);
    api
      .versions(form.core)
      .then((list) => {
        setVersions(list);
        setForm((f) => ({ ...f, version: list[0] ?? "" }));
      })
      .catch((e) => setError(e.message));
  }, [form.core]);

  async function submit(event: Event) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.createServer({
        name: form.name,
        core: form.core,
        version: form.version,
        java_major: form.java_major,
        port: form.port,
        memory: { min_mb: form.min_mb, max_mb: form.max_mb },
        eula_accepted: form.eula_accepted,
      });
      onCreated();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not create the server");
    } finally {
      setBusy(false);
    }
  }

  const set = (patch: Partial<typeof form>) => setForm((f) => ({ ...f, ...patch }));

  return (
    <Card title="New server">
      <form onSubmit={submit} class="space-y-5">
        {error && <Banner kind="error">{error}</Banner>}

        <div class="grid gap-4 sm:grid-cols-2">
          <Field label="Name">
            <Input
              value={form.name}
              placeholder="Survival"
              onInput={(e) => set({ name: (e.target as HTMLInputElement).value })}
            />
          </Field>

          <Field label="Port">
            <Input
              type="number"
              value={form.port}
              onInput={(e) => set({ port: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>

          <Field label="Flavour">
            <Select
              value={form.core}
              onChange={(e) => set({ core: (e.target as HTMLSelectElement).value })}
            >
              {providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.id}
                  {p.server ? "" : " (proxy)"}
                </option>
              ))}
            </Select>
          </Field>

          <Field label="Version" hint={versions.length === 0 ? "Loading…" : undefined}>
            <Select
              value={form.version}
              onChange={(e) => set({ version: (e.target as HTMLSelectElement).value })}
            >
              {versions.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </Select>
          </Field>

          <Field label="Java version" hint="Downloaded automatically if missing.">
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
        </div>

        <label class="flex items-start gap-2.5 text-sm">
          <input
            type="checkbox"
            checked={form.eula_accepted}
            onChange={(e) => set({ eula_accepted: (e.target as HTMLInputElement).checked })}
            class="mt-0.5 size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          <span class="text-fg-muted">
            I accept the{" "}
            <a
              href="https://aka.ms/MinecraftEULA"
              target="_blank"
              rel="noreferrer"
              class="text-accent underline"
            >
              Minecraft EULA
            </a>
            . The server will not start until this is accepted.
          </span>
        </label>

        <Button type="submit" variant="primary" disabled={busy || !form.version}>
          {busy ? "Creating…" : "Create server"}
        </Button>
      </form>
    </Card>
  );
}
