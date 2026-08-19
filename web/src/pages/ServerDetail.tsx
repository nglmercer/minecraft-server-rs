import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Backups } from "../components/Backups";
import { Console } from "../components/Console";
import { Files } from "../components/Files";
import { Mods } from "../components/Mods";
import { Banner, Button, Card, Field, Input, Select, StatusPill, formatUptime } from "../components/ui";
import { useDialogs } from "../components/Modal";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
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
  const t = useT();
  const toast = useToast();

  const [server, setServer] = useState<Server | null>(null);
  const [tab, setTab] = useState<Tab>("console");
  const [failed, setFailed] = useState<string | null>(null);

  async function refresh() {
    try {
      setServer(await api.server(id));
      setFailed(null);
    } catch (e) {
      setFailed(e instanceof Error ? e.message : t("errors.loadServer"));
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
    try {
      setServer(await api.power(id, action));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.actionFailed"));
    }
  }

  if (!server) {
    return (
      <div class="mx-auto max-w-6xl px-6 py-8">
        {failed ? (
          <Banner kind="error">{failed}</Banner>
        ) : (
          <p class="text-fg-muted">{t("common.loading")}</p>
        )}
      </div>
    );
  }

  const running = server.status !== "offline" && server.status !== "crashed";

  return (
    <div class="mx-auto flex h-full w-full max-w-6xl flex-col gap-5 px-4 py-6 sm:px-6">
      <header class="flex flex-wrap items-center justify-between gap-4">
        <div class="flex items-center gap-3">
          <Button variant="ghost" onClick={onBack}>
            ← {t("server.back")}
          </Button>
          <div>
            <h1 class="text-xl font-semibold">{server.name}</h1>
            <p class="text-xs text-fg-muted">
              {t("server.meta", {
                core: server.core,
                version: server.version,
                port: server.port,
                pid: server.pid ?? t("common.none"),
                uptime: formatUptime(server.uptime_secs),
              })}
              {server.metrics && (
                <>
                  {" · "}
                  <span class="tabular-nums">
                    {t("server.resources", {
                      cpu: server.metrics.cpu_percent.toFixed(0),
                      memory: server.metrics.memory_mb,
                    })}
                  </span>
                </>
              )}
            </p>
          </div>
        </div>

        <div class="flex flex-wrap items-center gap-x-3 gap-y-2">
          <StatusPill status={server.status} />
          <div class="flex flex-wrap items-center gap-2">
            <Button variant="primary" disabled={running} onClick={() => power("start")}>
              {t("dashboard.start")}
            </Button>
            <Button disabled={!running} onClick={() => power("restart")}>
              {t("dashboard.restart")}
            </Button>
            <Button variant="danger" disabled={!running} onClick={() => power("stop")}>
              {/* While provisioning there is nothing to shut down — only work to abandon. */}
              {server.status === "preparing" ? t("common.cancel") : t("dashboard.stop")}
            </Button>
            <Button variant="ghost" disabled={!running} onClick={() => power("kill")}>
              {t("dashboard.kill")}
            </Button>
          </div>
        </div>
      </header>

      {!server.eula_accepted && <Banner kind="info">{t("server.eulaWarning")}</Banner>}

      {server.status === "preparing" && <Banner kind="info">{t("server.preparing")}</Banner>}

      <nav class="-mx-4 flex gap-1 overflow-x-auto border-b border-ink-700 px-4 sm:mx-0 sm:px-0">
        {(["console", "files", "plugins", "backups", "settings"] as Tab[]).map((name) => (
          <button
            key={name}
            onClick={() => setTab(name)}
            class={`-mb-px shrink-0 whitespace-nowrap border-b-2 px-4 py-2 text-sm font-medium transition-colors ${
              tab === name
                ? "border-accent text-fg"
                : "border-transparent text-fg-muted hover:text-fg"
            }`}
          >
            {t(`server.tabs.${name}` as "server.tabs.console")}
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

/** What is installed on disk, and how to change it deliberately. */
function Installed({ server, onChanged }: { server: Server; onChanged: () => void }) {
  const t = useT();
  const toast = useToast();
  const dialogs = useDialogs();
  const [busy, setBusy] = useState(false);

  const running = server.status !== "offline" && server.status !== "crashed";

  async function update() {
    const confirmed = await dialogs.confirm({
      title: t("settings.updateTitle"),
      body: t("settings.updateBody"),
      confirmLabel: t("settings.update"),
    });
    if (!confirmed) return;

    setBusy(true);
    try {
      await api.reinstall(server.id);
      toast.success(t("settings.updated"));
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.actionFailed"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card title={t("settings.installSection")}>
      {server.installed ? (
        <div class="space-y-1">
          <p class="font-mono text-sm">
            {t("settings.installedAs", {
              core: server.installed.core,
              version: server.installed.version,
              build: server.installed.build,
              java: server.installed.java_major,
            })}
          </p>
          <p class="text-xs text-fg-muted">
            {t("settings.installedOn", {
              date: new Date(server.installed.installed_at).toLocaleString(),
            })}
          </p>
        </div>
      ) : (
        <p class="text-sm text-fg-muted">{t("settings.notInstalled")}</p>
      )}

      <p class="mt-3 text-xs leading-relaxed text-fg-muted">{t("settings.pinned")}</p>

      {server.needs_install && server.installed && (
        <div class="mt-3">
          <Banner kind="info">{t("settings.needsInstall")}</Banner>
        </div>
      )}

      <div class="mt-4">
        <Button
          disabled={busy || running}
          title={running ? t("settings.mustStopToUpdate") : undefined}
          onClick={update}
        >
          {busy ? t("settings.updating") : t("settings.update")}
        </Button>
      </div>
    </Card>
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
  const t = useT();
  const toast = useToast();
  const dialogs = useDialogs();
  const [busy, setBusy] = useState(false);

  const set = (patch: Partial<typeof form>) => setForm((f) => ({ ...f, ...patch }));

  async function submit(event: Event) {
    event.preventDefault();
    setBusy(true);
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
      toast.success(t("settings.saved"));
      onSaved();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    const confirmed = await dialogs.confirm({
      title: t("settings.removeTitle", { name: server.name }),
      body: t("settings.removeBody"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    try {
      await api.deleteServer(server.id);
      onDeleted();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    }
  }

  return (
    <form onSubmit={submit} class="space-y-5 overflow-y-auto pb-6">
      <Card title={t("settings.serverSection")}>
        <div class="grid gap-4 sm:grid-cols-2">
          <Field label={t("createServer.name")}>
            <Input value={form.name} onInput={(e) => set({ name: (e.target as HTMLInputElement).value })} />
          </Field>
          <Field label={t("createServer.port")}>
            <Input
              type="number"
              value={form.port}
              onInput={(e) => set({ port: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
          <Field label={t("createServer.javaVersion")}>
            <Select
              value={String(form.java_major)}
              onChange={(e) => set({ java_major: Number((e.target as HTMLSelectElement).value) })}
            >
              {[8, 11, 16, 17, 21, 25].map((v) => (
                <option key={v} value={v}>
                  {t("createServer.java", { version: v })}
                </option>
              ))}
            </Select>
          </Field>
          <div class="grid grid-cols-2 gap-3">
            <Field label={t("createServer.minRam")}>
              <Input
                type="number"
                value={form.min_mb}
                onInput={(e) => set({ min_mb: Number((e.target as HTMLInputElement).value) })}
              />
            </Field>
            <Field label={t("createServer.maxRam")}>
              <Input
                type="number"
                value={form.max_mb}
                onInput={(e) => set({ max_mb: Number((e.target as HTMLInputElement).value) })}
              />
            </Field>
          </div>
          <div class="sm:col-span-2">
            <Field label={t("settings.extraFlags")} hint={t("settings.extraFlagsHint")}>
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
          {t("settings.eulaAccepted")}
        </label>
      </Card>

      <Installed server={server} onChanged={onSaved} />

      <Card title={t("settings.recoverySection")}>
        <div class="grid gap-4 sm:grid-cols-3">
          <Field label={t("settings.maxRetries")}>
            <Input
              type="number"
              value={form.max_retries}
              onInput={(e) => set({ max_retries: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
          <Field label={t("settings.retryDelay")}>
            <Input
              type="number"
              value={form.retry_delay_secs}
              onInput={(e) => set({ retry_delay_secs: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>
          <Field label={t("settings.stopTimeout")}>
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
          {t("settings.autoRestart")}
        </label>
      </Card>

      <div class="flex items-center gap-3">
        <Button type="submit" variant="primary" disabled={busy}>
          {busy ? t("common.saving") : t("settings.saveChanges")}
        </Button>
        {user.admin && (
          <Button type="button" variant="danger" onClick={remove}>
            {t("settings.removeServer")}
          </Button>
        )}
      </div>
    </form>
  );
}
