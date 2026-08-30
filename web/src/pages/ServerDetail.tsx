import type { JSX } from "preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Backups } from "../components/Backups";
import { Console } from "../components/Console";
import { Files } from "../components/Files";
import { Mods } from "../components/Mods";
import {
  Banner,
  Button,
  Card,
  Field,
  IconButton,
  Input,
  Select,
  StatCard,
  StatusPill,
  formatBytes,
  formatUptime,
} from "../components/ui";
import * as Icon from "../components/icons";
import { useMenu } from "../components/Menu";
import { Tooltip } from "../components/Tooltip";
import { useDialogs } from "../components/Modal";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import type { Server, ServerPlayitView, Status, User } from "../types";

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
  const menu = useMenu();

  const [server, setServer] = useState<Server | null>(null);
  const [playit, setPlayit] = useState<ServerPlayitView | null>(null);
  const [tab, setTab] = useState<Tab>("console");
  const [failed, setFailed] = useState<string | null>(null);
  const [progress, setProgress] = useState<{ stage: string; fraction: number | null } | null>(null);

  async function refresh() {
    try {
      const [nextServer, nextPlayit] = await Promise.all([api.server(id), api.serverPlayit(id)]);
      setServer(nextServer);
      setPlayit(nextPlayit);
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
    if (status !== "preparing") setProgress(null);
  }

  function onProgress(p: { stage: string; fraction: number | null } | null) {
    setProgress(p);
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

  const tabs: { id: Tab; icon: JSX.Element }[] = [
    { id: "console", icon: <Icon.Terminal size={15} /> },
    { id: "files", icon: <Icon.Folder size={15} /> },
    { id: "plugins", icon: <Icon.Package size={15} /> },
    { id: "backups", icon: <Icon.Archive size={15} /> },
    { id: "settings", icon: <Icon.Settings size={15} /> },
  ];

  const memoryFraction = server.metrics
    ? server.metrics.memory_mb / server.memory.max_mb
    : 0;

  return (
    <div class="mx-auto flex h-full w-full max-w-6xl flex-col gap-5 px-4 py-6 sm:px-6">
      <header class="space-y-4">
        <div class="flex flex-wrap items-start justify-between gap-4">
          <div class="flex min-w-0 items-start gap-4">
            {/* A stable identity tile, coloured from the id so servers stay
                visually distinguishable in a list of similar names. */}
            <div
              class="grid size-14 shrink-0 place-items-center rounded-xl border border-ink-700 text-xl font-semibold"
              style={{ background: tileColour(server.id) }}
              aria-hidden="true"
            >
              {server.name.slice(0, 1).toUpperCase()}
            </div>

            <div class="min-w-0">
              <button
                onClick={onBack}
                class="inline-flex items-center gap-1 text-sm text-accent hover:underline"
              >
                <Icon.ArrowLeft size={14} />
                {t("server.back")}
              </button>

              <h1 class="mt-0.5 truncate text-2xl font-semibold tracking-tight">
                {server.name}
              </h1>

              <div class="mt-1.5 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-fg-muted">
                <Meta icon={<Icon.Gamepad size={14} />} label={t("server.metaVersion")}>
                  Minecraft {server.version}
                </Meta>
                <Divider />
                <Meta icon={<Icon.Tag size={14} />} label={t("server.metaCore")}>
                  {server.core}
                  {server.installed && ` ${server.installed.build}`}
                </Meta>
                <Divider />
                <Meta icon={<Icon.Link size={14} />} label={t("server.metaPort")}>
                  :{server.port}
                </Meta>
                <Divider />
                <Meta icon={<Icon.Clock size={14} />} label={t("server.metaUptime")}>
                  {formatUptime(server.uptime_secs)}
                </Meta>
              </div>
            </div>
          </div>

          <div class="flex items-center gap-2">
            <StatusPill status={server.status} />

            {running ? (
              <Button
                variant="ghost"
                icon={<Icon.Stop size={15} />}
                onClick={() => power("stop")}
              >
                {server.status === "preparing" ? t("common.cancel") : t("dashboard.stop")}
              </Button>
            ) : (
              <Button
                variant="primary"
                icon={<Icon.Play size={13} />}
                onClick={() => power("start")}
              >
                {t("dashboard.start")}
              </Button>
            )}

            <Button
              variant="primary"
              icon={<Icon.Restart size={15} />}
              disabled={!running}
              onClick={() => power("restart")}
            >
              {t("dashboard.restart")}
            </Button>

            <IconButton
              label={t("common.more")}
              icon={<Icon.Dots size={18} />}
              onClick={(event) =>
                menu.open(
                  event as unknown as MouseEvent,
                  [
                    {
                      label: t("dashboard.start"),
                      onSelect: () => power("start"),
                      disabled: running,
                    },
                    {
                      label: t("dashboard.restart"),
                      onSelect: () => power("restart"),
                      disabled: !running,
                    },
                    {
                      label: t("dashboard.stop"),
                      onSelect: () => power("stop"),
                      disabled: !running,
                    },
                    {
                      label: t("dashboard.kill"),
                      danger: true,
                      onSelect: () => power("kill"),
                      disabled: !running,
                    },
                  ],
                  server.name,
                )
              }
            />
          </div>
        </div>

        <nav class="-mx-4 flex gap-1 overflow-x-auto px-4 sm:mx-0 sm:px-0">
          <div class="flex gap-1 rounded-full border border-ink-700 bg-ink-850 p-1">
            {tabs.map(({ id, icon }) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                class={`inline-flex shrink-0 items-center gap-2 whitespace-nowrap rounded-full px-4 py-1.5 text-sm font-medium transition-colors ${
                  tab === id
                    ? "bg-accent text-ink-950"
                    : "text-fg-muted hover:bg-ink-700 hover:text-fg"
                }`}
              >
                {icon}
                {t(`server.tabs.${id}` as "server.tabs.console")}
              </button>
            ))}
          </div>
        </nav>
      </header>

      {!server.eula_accepted && <Banner kind="info">{t("server.eulaWarning")}</Banner>}

      {server.status === "preparing" && (
        <div class="rounded-lg border border-sky-500/40 bg-sky-500/10 px-4 py-3">
          <p class="text-sm font-medium text-sky-100">{progress?.stage ?? t("server.preparing")}</p>
          {progress?.fraction !== null && progress?.fraction !== undefined && (
            <div class="mt-2 flex items-center gap-3">
              <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-sky-900">
                <div
                  class="h-full bg-sky-400 transition-[width] duration-300"
                  style={{ width: `${Math.max(0, Math.min(1, progress.fraction ?? 0)) * 100}%` }}
                />
              </div>
              <span class="shrink-0 text-xs tabular-nums text-sky-100">
                {Math.round((progress?.fraction ?? 0) * 100)}%
              </span>
            </div>
          )}
        </div>
      )}

      {tab === "console" && (
        <div class="grid gap-4 sm:grid-cols-3">
          <StatCard
            icon={<Icon.Cpu size={20} />}
            value={`${(server.metrics?.cpu_percent ?? 0).toFixed(2)}%`}
            max="100%"
            label={t("server.cpuUsage")}
            fraction={(server.metrics?.cpu_percent ?? 0) / 100}
          />
          <StatCard
            icon={<Icon.Memory size={20} />}
            value={`${server.metrics?.memory_mb ?? 0} MB`}
            max={`${server.memory.max_mb} MB`}
            label={t("server.memoryUsage")}
            fraction={memoryFraction}
            tone={memoryFraction > 0.9 ? "warn" : "accent"}
          />
          <StatCard
            icon={<Icon.Folder size={20} />}
            value={formatBytes(server.disk_bytes)}
            label={t("server.storageUsage")}
          />
        </div>
      )}

      <div class="flex min-h-0 flex-1 flex-col">
        {tab === "console" && <Console serverId={id} onStatus={onStatus} onProgress={onProgress} />}
        {tab === "files" && <Files serverId={id} />}
        {tab === "plugins" && <Mods server={server} />}
        {tab === "backups" && <Backups serverId={id} status={server.status} />}
        {tab === "settings" && (
          <Settings
            server={server}
            playit={playit}
            user={user}
            onSaved={refresh}
            onDeleted={onBack}
          />
        )}
      </div>
    </div>
  );
}

/** What is installed on disk, and how to change it deliberately. */
/** A labelled metadata chip in the header. */
function Meta({
  icon,
  label,
  children,
}: {
  icon: JSX.Element;
  label: string;
  children: preact.ComponentChildren;
}) {
  return (
    <Tooltip label={label}>
      <span class="inline-flex items-center gap-1.5">
        <span class="text-fg-muted/70">{icon}</span>
        {children}
      </span>
    </Tooltip>
  );
}

function Divider() {
  return <span class="hidden h-3 w-px bg-ink-600 sm:block" aria-hidden="true" />;
}

/** A stable tint per server, so two similarly named servers still look different. */
function tileColour(id: string): string {
  let hash = 0;
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) % 360;
  return `linear-gradient(140deg, hsl(${hash} 45% 22%), hsl(${(hash + 40) % 360} 45% 14%))`;
}

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

  async function prepare() {
    setBusy(true);
    try {
      await api.prepare(server.id);
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

      {server.pending_restart && (
        <div class="mt-3">
          <Banner kind="info">{t("settings.pendingRestart")}</Banner>
        </div>
      )}

      <div class="mt-4 flex flex-wrap gap-2">
        {!server.installed && (
          <Button
            variant="primary"
            disabled={busy || running}
            title={running ? t("settings.mustStopToUpdate") : undefined}
            onClick={prepare}
          >
            {busy ? t("settings.updating") : "Install"}
          </Button>
        )}
        {server.needs_install && (
          <Button
            variant="primary"
            disabled={busy || running}
            title={running ? t("settings.mustStopToUpdate") : undefined}
            onClick={prepare}
          >
            {busy ? t("settings.updating") : "Install"}
          </Button>
        )}
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
  playit,
  user,
  onSaved,
  onDeleted,
}: {
  server: Server;
  playit: ServerPlayitView | null;
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
      const update: Record<string, unknown> = {
        name: form.name,
        port: form.port,
        java_major: form.java_major,
        memory: { min_mb: form.min_mb, max_mb: form.max_mb },
        eula_accepted: form.eula_accepted,
        policy: {
          ...server.policy,
          auto_restart: form.auto_restart,
          max_retries: form.max_retries,
          retry_delay_secs: form.retry_delay_secs,
          stop_timeout_secs: form.stop_timeout_secs,
        },
      };
      if (user.admin) {
        update.jvm_args = form.jvm_args.split(/\s+/).filter(Boolean);
      }
      await api.updateServer(server.id, update);
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
          {user.admin && (
            <div class="sm:col-span-2">
              <Field label={t("settings.extraFlags")} hint={t("settings.extraFlagsHint")}>
                <Input
                  value={form.jvm_args}
                  placeholder="-XX:+UseG1GC -XX:MaxGCPauseMillis=200"
                  onInput={(e) => set({ jvm_args: (e.target as HTMLInputElement).value })}
                />
              </Field>
            </div>
          )}
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

      {playit && <PlayitSettings server={server} playit={playit} user={user} onChanged={onSaved} />}

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

function PlayitSettings({
  server,
  playit,
  user,
  onChanged,
}: {
  server: Server;
  playit: ServerPlayitView;
  user: User;
  onChanged: () => void;
}) {
  const t = useT();
  const toast = useToast();
  const dialogs = useDialogs();
  const [busy, setBusy] = useState(false);

  async function attach() {
    setBusy(true);
    try {
      await api.attachPlayit(server.id);
      toast.success(t("playit.tunnelCreated"));
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.playitAction"));
    } finally {
      setBusy(false);
    }
  }

  async function detach() {
    const confirmed = await dialogs.confirm({
      title: t("playit.deleteTunnelTitle", { name: server.name }),
      body: t("playit.detachTunnelBody"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    setBusy(true);
    try {
      await api.detachPlayit(server.id);
      toast.success(t("playit.tunnelDeleted"));
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.playitAction"));
    } finally {
      setBusy(false);
    }
  }

  async function copyAddress() {
    const address = playit.tunnel?.display_address;
    if (!address || !navigator.clipboard) {
      toast.error(t("playit.copyUnavailable"));
      return;
    }
    try {
      await navigator.clipboard.writeText(address);
      toast.success(t("playit.addressCopied"));
    } catch {
      toast.error(t("playit.copyUnavailable"));
    }
  }

  const stateKind = ["unavailable", "drifted", "disabled_by_playit"].includes(playit.state)
    ? "error"
    : "info";

  return (
    <Card title={t("playit.serverCardTitle")}>
      <div class="flex flex-wrap items-center justify-between gap-3">
        <div>
          <p class="font-medium">{t(`playit.serverStates.${playit.state}` as "playit.serverStates.disabled")}</p>
          <p class="mt-1 text-sm text-fg-muted">
            {playit.binding
              ? `${playit.binding.local_address}:${playit.binding.local_port}`
              : t("playit.serverNotConfigured")}
          </p>
        </div>
        {playit.tunnel?.display_address && (
          <Button variant="ghost" icon={<Icon.Copy size={15} />} onClick={() => void copyAddress()}>
            {playit.tunnel.display_address}
          </Button>
        )}
      </div>

      {playit.message && (
        <div class="mt-4">
          <Banner kind={stateKind}>{playit.message}</Banner>
        </div>
      )}

      {playit.tunnel && (
        <p class="mt-3 text-xs text-fg-muted">
          {t("playit.destination")}: <span class="font-mono text-fg">{playit.tunnel.destination}</span>
        </p>
      )}

      {user.admin && (
        <div class="mt-4 flex flex-wrap gap-2">
          {playit.state === "disabled" && (
            <Button variant="primary" disabled={busy} onClick={() => void attach()}>
              {busy ? t("common.creating") : t("playit.connectServer")}
            </Button>
          )}
          {playit.state !== "disabled" && (
            <Button variant="danger" disabled={busy} onClick={() => void detach()}>
              {busy ? t("common.deleting") : t("playit.disconnectServer")}
            </Button>
          )}
        </div>
      )}
    </Card>
  );
}
