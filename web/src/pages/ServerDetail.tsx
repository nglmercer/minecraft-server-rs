import type { JSX } from "preact";
import { lazy, Suspense } from "preact/compat";
import { Loading } from "../components/Loading";
import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { useServerPower } from "../useServerPower";
import { Guide } from "../components/Guide";
import { Console } from "../components/Console";
import { ServerActivity } from "../components/ServerActivity";
import {
  Banner,
  Button,
  IconButton,
  StatCard,
  StatusPill,
  formatBytes,
  formatUptime,
} from "../components/ui";
import * as Icon from "../components/icons";
import { tileColour } from "../components/serverTile";
import { useMenu } from "../components/Menu";
import { Tooltip } from "../components/Tooltip";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import { serverActionCapabilities } from "../serverActions";
import type {
  ProgressState,
  Server,
  ServerPlayitView,
  Status,
  User,
} from "../types";

const Backups = lazy(() => import("../components/Backups").then(m => ({ default: m.Backups })));
const Files = lazy(() => import("../components/Files").then(m => ({ default: m.Files })));
const Mods = lazy(() => import("../components/Mods").then(m => ({ default: m.Mods })));
const ServerBackupSettings = lazy(() => import("../components/ServerBackupSettings").then(m => ({ default: m.ServerBackupSettings })));
const Settings = lazy(() => import("./ServerSettings").then(m => ({ default: m.Settings })));

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
        <Suspense fallback={<Loading />}>
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
        </Suspense>
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
