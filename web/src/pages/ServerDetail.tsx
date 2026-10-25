import type { JSX } from "preact";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { useServerPower } from "../useServerPower";
import { Guide } from "../components/Guide";
import { Compatibility } from "../components/Compatibility";
import { ChangeServerVersion } from "../components/ChangeServerVersion";
import { Backups } from "../components/Backups";
import { Console } from "../components/Console";
import { Files } from "../components/Files";
import { Mods } from "../components/Mods";
import { ServerActivity } from "../components/ServerActivity";
import { ServerBackupSettings } from "../components/ServerBackupSettings";
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
import { tileColour } from "../components/serverTile";
import { useMenu } from "../components/Menu";
import { Tooltip } from "../components/Tooltip";
import { useDialogs } from "../components/Modal";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import { serverActionCapabilities } from "../serverActions";
import type {
  PlayitAttachDisposition,
  ProgressState,
  Server,
  ServerPlayitView,
  Status,
  User,
} from "../types";

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
  const guardedPower = useServerPower();

  const [server, setServer] = useState<Server | null>(null);
  const [playit, setPlayit] = useState<ServerPlayitView | null>(null);
  const [tab, setTab] = useState<Tab>("console");
  const [failed, setFailed] = useState<string | null>(null);
  const [progress, setProgress] = useState<ProgressState | null>(null);

  async function refresh() {
    try {
      const [nextServer, nextPlayit] = await Promise.all([api.server(id), api.serverPlayit(id)]);
      setServer(nextServer);
      setPlayit(nextPlayit);
      setProgress(nextServer.status === "preparing" ? nextServer.progress : null);
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
    // A new lifecycle begins with a fresh snapshot/progress event. Clear any
    // previous run immediately so a delayed event cannot bleed into it.
    setProgress(null);
  }

  function onProgress(p: ProgressState | null) {
    setProgress(p);
  }

  async function power(action: "start" | "stop" | "restart" | "kill") {
    try {
      if (!server) return;
      const nextServer = await guardedPower(server, action);
      if (!nextServer) return;
      setServer(nextServer);
      setProgress(nextServer.status === "preparing" ? nextServer.progress : null);
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

  const actions = serverActionCapabilities(server.status);

  const tabs: { id: Tab; icon: JSX.Element }[] = [
    { id: "console", icon: <Icon.Terminal size={15} /> },
    { id: "files", icon: <Icon.Folder size={15} /> },
    { id: "plugins", icon: <Icon.Package size={15} /> },
    { id: "backups", icon: <Icon.Archive size={15} /> },
    { id: "settings", icon: <Icon.Settings size={15} /> },
  ];

  const cpuHostPercent =
    typeof server.metrics?.cpu_host_percent === "number" &&
    Number.isFinite(server.metrics.cpu_host_percent)
      ? Math.max(0, Math.min(100, server.metrics.cpu_host_percent))
      : null;
  const cpuCores =
    typeof server.metrics?.cpu_cores === "number" && Number.isFinite(server.metrics.cpu_cores)
      ? server.metrics.cpu_cores
      : null;
  const logicalCpuCount =
    typeof server.metrics?.logical_cpu_count === "number" && server.metrics.logical_cpu_count > 0
      ? server.metrics.logical_cpu_count
      : null;
  const hasCpuMetric = cpuHostPercent !== null && cpuCores !== null && logicalCpuCount !== null;
  const memoryMb = server.metrics?.memory_mb;
  const memoryFraction =
    typeof memoryMb === "number" && server.memory.max_mb > 0
      ? Math.max(0, Math.min(1, memoryMb / server.memory.max_mb))
      : undefined;

  return (
    <div
      data-testid="server-detail"
      data-server-id={server.id}
      class="mx-auto flex h-full w-full max-w-6xl flex-col gap-3 px-3 py-3 sm:gap-5 sm:px-6 sm:py-6"
    >
      <header class="space-y-3 sm:space-y-4">
        <div class="rounded-2xl border border-ink-700 bg-ink-850 p-3 sm:p-5">
          <div class="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between">
            <div class="flex min-w-0 items-start gap-3 sm:gap-4">
            {/* A stable identity tile, coloured from the id so servers stay
                visually distinguishable in a list of similar names. */}
            <div
              class="grid size-12 shrink-0 place-items-center rounded-xl border border-ink-700 text-lg font-semibold sm:size-14 sm:text-xl"
              style={{ background: tileColour(server.id ?? id) }}
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

              <h1 class="mt-0.5 truncate text-xl font-semibold tracking-tight sm:text-2xl">
                {server.name}
              </h1>

              <div class="mt-1.5 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-fg-muted sm:gap-x-3">
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
                {server.uptime_secs !== null && (
                  <>
                    <Divider />
                    <Meta icon={<Icon.Clock size={14} />} label={t("server.metaUptime")}>
                      {formatUptime(server.uptime_secs)}
                    </Meta>
                  </>
                )}
              </div>
            </div>
          </div>

            <div class="flex min-w-0 items-center justify-between gap-1 sm:justify-end sm:gap-2">
            <div data-testid="server-status" data-status={server.status}>
              <StatusPill status={server.status} />
            </div>

            {actions.cancel ? (
              <Button
                variant="ghost"
                icon={<Icon.Stop size={15} />}
                onClick={() => power("stop")}
              >
                {t("common.cancel")}
              </Button>
            ) : actions.stop ? (
              <Button
                variant="ghost"
                icon={<Icon.Stop size={15} />}
                onClick={() => power("stop")}
              >
                {t("dashboard.stop")}
              </Button>
            ) : actions.start ? (
              <Button
                variant="primary"
                icon={<Icon.Play size={13} />}
                data-testid="server-start"
                onClick={() => power("start")}
              >
                {t("dashboard.start")}
              </Button>
            ) : null}

            <Button
              variant="primary"
              icon={<Icon.Restart size={15} />}
              disabled={!actions.restart}
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
                      disabled: !actions.start,
                    },
                    {
                      label: t("dashboard.restart"),
                      onSelect: () => power("restart"),
                      disabled: !actions.restart,
                    },
                    {
                      label: actions.cancel ? t("common.cancel") : t("dashboard.stop"),
                      onSelect: () => power("stop"),
                      disabled: !(actions.stop || actions.cancel),
                    },
                    {
                      label: t("dashboard.kill"),
                      danger: true,
                      onSelect: () => power("kill"),
                      disabled: !actions.kill,
                    },
                  ],
                  server.name,
                )
              }
            />
            </div>
          </div>
        </div>

        <nav class="-mx-3 flex px-3 sm:mx-0 sm:px-0">
          <div class="flex w-full gap-0.5 rounded-2xl border border-ink-700 bg-ink-850 p-1 sm:w-fit sm:rounded-full">
            {tabs.map(({ id, icon }) => {
              const label = t(`server.tabs.${id}` as "server.tabs.console");
              return (
                <Tooltip key={id} label={label} wrapperClass="min-w-0 flex-1 sm:flex-none">
                  <button
                    type="button"
                    aria-label={label}
                    aria-current={tab === id ? "page" : undefined}
                    title={label}
                    onClick={() => setTab(id)}
                    class={`inline-flex h-10 w-full min-w-0 items-center justify-center rounded-xl px-1.5 text-sm font-medium transition-colors sm:h-auto sm:w-auto sm:gap-2 sm:rounded-full sm:px-4 sm:py-1.5 ${
                      tab === id
                        ? "bg-accent text-ink-950"
                        : "text-fg-muted hover:bg-ink-700 hover:text-fg"
                    }`}
                  >
                    {icon}
                    <span class="hidden whitespace-nowrap sm:inline">{label}</span>
                  </button>
                </Tooltip>
              );
            })}
          </div>
        </nav>
      </header>

      <Guide steps={tabs.map(({ id }) => ({
        title: t(`server.tabs.${id}` as "server.tabs.console"),
        body: t(`guide.${id}` as "guide.console"),
        enter: () => setTab(id),
      }))} />
      <ServerActivity status={server.status} progress={progress} />

      {!server.eula_accepted && (
        <Banner kind="info">
          <div class="flex flex-wrap items-center justify-between gap-3">
            <span>{t("server.eulaWarning")}</span>
            <Button disabled={!actions.start} onClick={() => void power("start")}>
              {t("launch.acceptAndStart")}
            </Button>
          </div>
        </Banner>
      )}

      {tab === "console" && (
        <div class="grid grid-cols-3 gap-2 sm:gap-4">
          <StatCard
            compact
            shortLabel={t("server.cpuShort")}
            icon={<Icon.Cpu size={18} />}
            value={hasCpuMetric ? `${cpuHostPercent.toFixed(2)}%` : "—"}
            max={hasCpuMetric ? "100%" : undefined}
            label={t("server.cpuUsage")}
            fraction={hasCpuMetric ? cpuHostPercent / 100 : undefined}
            detail={
              hasCpuMetric
                ? t("server.cpuCores", {
                    cores: cpuCores.toFixed(2),
                    count: logicalCpuCount,
                  })
                : t("server.notRunning")
            }
          />
          <StatCard
            compact
            shortLabel={t("server.memoryShort")}
            icon={<Icon.Memory size={18} />}
            value={memoryMb === undefined ? "—" : `${memoryMb} MiB`}
            label={t("server.memoryUsage")}
            fraction={memoryFraction}
            tone={memoryFraction !== undefined && memoryFraction > 0.9 ? "warn" : "accent"}
            detail={
              memoryMb === undefined
                ? t("server.notRunning")
                : t("server.heapMax", { memory: server.memory.max_mb })
            }
          />
          <StatCard
            compact
            shortLabel={t("server.storageShort")}
            icon={<Icon.Folder size={18} />}
            value={formatBytes(server.disk_bytes)}
            label={t("server.storageUsage")}
          />
        </div>
      )}

      <div class="flex min-h-0 flex-1 flex-col">
        {tab === "console" && (
          <Console
            serverId={id}
            status={server.status}
            progress={progress}
            onStatus={onStatus}
            onProgress={onProgress}
          />
        )}
        {tab === "files" && <Files serverId={id} />}
        {tab === "plugins" && <Mods server={server} />}
        {tab === "backups" && (
          <div class="space-y-6">
            {user.admin && <ServerBackupSettings serverId={id} />}
            <Backups serverId={id} status={server.status} />
          </div>
        )}
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

function Installed({ server, onChanged }: { server: Server; onChanged: () => void }) {
  const t = useT();
  const toast = useToast();
  const dialogs = useDialogs();
  const [busy, setBusy] = useState(false);
  const [compatible, setCompatible] = useState(false);
  const [changingVersion, setChangingVersion] = useState(false);

  const lifecycleBusy = server.status !== "offline" && server.status !== "crashed";

  async function update() {
    if (busy || lifecycleBusy || !compatible) return;
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
    if (busy || lifecycleBusy || !compatible) return;
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

      <div class="mt-4 border-t border-ink-700 pt-3">
        <Compatibility core={server.core} version={server.version} javaMajor={server.java_major} onAllowedChange={setCompatible} />
      </div>

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

      <div class="mt-4 flex flex-col gap-2 sm:flex-row">
        <Button disabled={busy || lifecycleBusy} onClick={() => setChangingVersion(true)}
          title={lifecycleBusy ? t("settings.mustStopToUpdate") : undefined}>
          {t("settings.changeVersion")}
        </Button>
        {/* One button for both cases: a fresh server has nothing installed
            and always needs an install, so two separate conditions rendered
            the same Install button twice. */}
        {(!server.installed || server.needs_install) && (
          <Button
            variant="primary"
            class="w-full sm:w-auto"
            disabled={busy || lifecycleBusy || !compatible}
            title={lifecycleBusy ? t("settings.mustStopToUpdate") : undefined}
            onClick={prepare}
          >
            {busy ? t("settings.updating") : t("common.install")}
          </Button>
        )}
        {!server.needs_install && server.installed && <Button
          class="w-full sm:w-auto"
          disabled={busy || lifecycleBusy || !compatible}
          title={lifecycleBusy ? t("settings.mustStopToUpdate") : undefined}
          onClick={update}
        >
          {busy ? t("settings.updating") : t("settings.update")}
        </Button>}
      </div>
      {changingVersion && <ChangeServerVersion server={server} onClose={() => setChangingVersion(false)} onSaved={onChanged} />}
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
    <form onSubmit={submit} class="min-w-0 space-y-3 overflow-y-auto pb-6 sm:space-y-5">
      <Card title={t("settings.serverSection")} class="overflow-hidden">
        <div class="grid gap-3 sm:gap-4 sm:grid-cols-2">
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

        <label class="mt-3 flex items-start gap-2.5 border-t border-ink-700 pt-3 text-sm text-fg-muted sm:mt-4 sm:items-center">
          <input
            type="checkbox"
            checked={form.eula_accepted}
            onChange={(e) => set({ eula_accepted: (e.target as HTMLInputElement).checked })}
            class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          {t("settings.eulaAccepted")}
        </label>
      </Card>

      {playit && (
        <details class="rounded-xl border border-ink-700 p-4">
          <summary class="cursor-pointer font-semibold">{t("nav.playit")}</summary>
          <div class="mt-4">
            <PlayitSettings server={server} playit={playit} user={user} onChanged={onSaved} />
          </div>
        </details>
      )}

      <Installed server={server} onChanged={onSaved} />

      <details class="rounded-xl border border-ink-700 p-4">
        <summary class="cursor-pointer font-semibold">{t("settings.recoverySection")}</summary>
        <div class="mt-4">
          <div class="grid gap-3 sm:gap-4 sm:grid-cols-3">
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
          <label class="mt-3 flex items-start gap-2.5 border-t border-ink-700 pt-3 text-sm text-fg-muted sm:mt-4 sm:items-center">
            <input
              type="checkbox"
              checked={form.auto_restart}
              onChange={(e) => set({ auto_restart: (e.target as HTMLInputElement).checked })}
              class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
            />
            {t("settings.autoRestart")}
          </label>

      </div>
      </details>

      <div class="flex flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
        <Button type="submit" variant="primary" class="w-full sm:w-auto" disabled={busy}>
          {busy ? t("common.saving") : t("settings.saveChanges")}
        </Button>
        {user.admin && (
          <Button type="button" variant="danger" class="w-full sm:w-auto" onClick={remove}>
            {t("settings.removeServer")}
          </Button>
        )}
      </div>
    </form>
  );
}

export function PlayitSettings({
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
      const view = await api.attachPlayit(server.id);
      toast.success(attachSuccessMessage(view.disposition, t));
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
      const result = await api.detachPlayit(server.id);
      if (result.cleanup_pending) toast.info(t("playit.tunnelCleanupPending"));
      else toast.success(t("playit.tunnelDeleted"));
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.playitAction"));
    } finally {
      setBusy(false);
    }
  }

  async function reconcile() {
    setBusy(true);
    try {
      await api.reconcilePlayit(server.id);
      toast.success(t("playit.serverReconciled"));
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.playitAction"));
    } finally {
      setBusy(false);
    }
  }

  async function repair() {
    setBusy(true);
    try {
      const view = await api.attachPlayit(server.id);
      toast.success(attachSuccessMessage(view.disposition, t));
      onChanged();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.playitAction"));
    } finally {
      setBusy(false);
    }
  }

  async function forget() {
    const confirmed = await dialogs.confirm({
      title: t("playit.forgetServer"),
      body: t("playit.forgetServerBody"),
      confirmLabel: t("playit.forgetServer"),
    });
    if (!confirmed) return;

    setBusy(true);
    try {
      await api.forgetPlayit(server.id);
      toast.success(t("playit.serverAssociationForgotten"));
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

  const recoveryStates = [
    "missing",
    "drifted",
    "unavailable",
    "account_mismatch",
    "agent_mismatch",
    "ambiguous",
    "disabled_by_playit",
    "reconnecting",
  ];
  const repairStates = ["missing", "drifted", "agent_mismatch"];
  const cleanupPending = playit.cleanup_pending;
  const needsReconcile = cleanupPending || recoveryStates.includes(playit.state);
  const stateKind = recoveryStates.includes(playit.state)
    ? "error"
    : "info";

  return (
    <Card title={t("playit.serverCardTitle")} class="overflow-hidden">
      <div class="grid gap-2 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center sm:gap-3">
        <div class="min-w-0 rounded-xl border border-ink-700 bg-ink-900/45 px-3 py-2.5">
          <div class="flex items-center gap-2">
            <span
              class={`size-2 shrink-0 rounded-full ${recoveryStates.includes(playit.state) ? "bg-red-400" : playit.state === "connected" ? "bg-accent" : "bg-amber-400"}`}
              aria-hidden="true"
            />
            <p class="truncate font-medium">{t(`playit.serverStates.${playit.state}` as "playit.serverStates.disabled")}</p>
          </div>
          <p class="mt-1 truncate text-sm text-fg-muted">
            {playit.binding
              ? `${playit.binding.local_address}:${playit.binding.local_port}`
              : t("playit.serverNotConfigured")}
          </p>
        </div>
        {playit.tunnel?.display_address && (
          <Button
            variant="ghost"
            icon={<Icon.Copy size={15} />}
            aria-label={t("playit.copyAddress")}
            title={t("playit.copyAddress")}
            class="w-full justify-start sm:w-auto sm:max-w-full"
            onClick={() => void copyAddress()}
          >
            <span class="truncate">{playit.tunnel.display_address}</span>
          </Button>
        )}
      </div>

      {playit.message && (
        <div class="mt-4">
          <Banner kind={stateKind}>{playit.message}</Banner>
        </div>
      )}

      {playit.tunnel && (
        <p class="mt-3 truncate text-xs text-fg-muted">
          {t("playit.destination")}: <span class="font-mono text-fg">{playit.tunnel.destination}</span>
        </p>
      )}

      {user.admin && (
        <div class="mt-4 flex flex-col gap-2 sm:flex-row sm:flex-wrap">
          {playit.state === "disabled" && !cleanupPending && (
            <Button variant="primary" class="w-full sm:w-auto" disabled={busy} onClick={() => void attach()}>
              {busy ? t("common.creating") : t("playit.connectServer")}
            </Button>
          )}
          {needsReconcile && (
            <Button variant="ghost" class="w-full sm:w-auto" disabled={busy} onClick={() => void reconcile()}>
              {t("playit.reconcileServer")}
            </Button>
          )}
          {repairStates.includes(playit.state) && (
            <Button variant="primary" class="w-full sm:w-auto" disabled={busy} onClick={() => void repair()}>
              {busy ? t("common.creating") : t("playit.repairServer")}
            </Button>
          )}
          {playit.state !== "disabled" && (
            <Button variant="danger" class="w-full sm:w-auto" disabled={busy} onClick={() => void detach()}>
              {busy ? t("common.deleting") : t("playit.disconnectServer")}
            </Button>
          )}
          {(recoveryStates.includes(playit.state) || cleanupPending) && (
            <Button variant="subtle" class="w-full sm:w-auto" disabled={busy} onClick={() => void forget()}>
              {t("playit.forgetServer")}
            </Button>
          )}
        </div>
      )}
    </Card>
  );
}

function attachSuccessMessage(
  disposition: PlayitAttachDisposition | null | undefined,
  t: ReturnType<typeof useT>,
): string {
  if (disposition === "reused") return t("playit.tunnelReused");
  if (disposition === "updated") return t("playit.tunnelUpdated");
  return t("playit.tunnelCreated");
}
