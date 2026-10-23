import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { Modal } from "../components/Modal";
import { Button, Card, Field, Input, Select, StatCard, StatusPill, formatUptime } from "../components/ui";
import * as Icon from "../components/icons";
import { tileColour } from "../components/serverTile";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import { recommendedJavaForVersion } from "../minecraftJava";
import { serverActionCapabilities } from "../serverActions";
import type { Server, SystemStats, User } from "../types";

export function Dashboard({
  user,
  onOpen,
}: {
  user: User;
  onOpen: (id: string) => void;
}) {
  const t = useT();
  const toast = useToast();

  const [servers, setServers] = useState<Server[]>([]);
  const [stats, setStats] = useState<SystemStats | null>(null);
  const [creating, setCreating] = useState(false);

  async function refresh() {
    try {
      const [list, system] = await Promise.all([api.servers(), api.system()]);
      setServers(list);
      setStats(system);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.loadServers"));
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
      toast.error(e instanceof Error ? e.message : t("errors.actionFailed"));
    }
  }

  const cpuFraction = Math.max(0, Math.min(1, (stats?.cpu_percent ?? 0) / 100));
  const memoryFraction = stats
    ? Math.max(0, Math.min(1, stats.memory_used_mb / Math.max(1, stats.memory_total_mb)))
    : 0;
  const onlineFraction = stats
    ? Math.max(0, Math.min(1, stats.servers_online / Math.max(1, servers.length)))
    : 0;

  return (
    <div class="mx-auto flex w-full max-w-6xl flex-col gap-3 px-3 py-3 sm:gap-6 sm:px-6 sm:py-8">
      <header class="flex items-center justify-between gap-3 rounded-2xl border border-ink-700 bg-ink-850 px-3 py-3 sm:items-end sm:border-0 sm:bg-transparent sm:px-0 sm:py-0">
        <div class="min-w-0">
          <h1 class="truncate text-xl font-semibold sm:text-2xl">{t("dashboard.title")}</h1>
          <p class="text-sm text-fg-muted">
            {t("dashboard.summary", {
              count: servers.length,
              online: stats?.servers_online ?? 0,
            })}
          </p>
        </div>
        {user.admin && (
          <Button
            variant="primary"
            icon={<Icon.Plus size={15} />}
            class="shrink-0"
            data-testid="new-server"
            onClick={() => setCreating(true)}
          >
            {t("dashboard.newServer")}
          </Button>
        )}
      </header>

      {stats && (
        <div class="grid grid-cols-3 gap-2 sm:gap-4">
          <StatCard
            compact
            shortLabel={t(stats.cpu_estimated ? "dashboard.hostCpuEstimatedShort" : "dashboard.hostCpuShort")}
            icon={<Icon.Cpu size={18} />}
            value={stats.cpu_percent === null ? t("common.unknown") : `${stats.cpu_percent.toFixed(0)}%`}
            max="100%"
            label={t(stats.cpu_estimated ? "dashboard.hostCpuEstimated" : "dashboard.hostCpu")}
            fraction={cpuFraction}
          />
          <StatCard
            compact
            shortLabel={t("dashboard.hostMemoryShort")}
            icon={<Icon.Memory size={18} />}
            value={`${(stats.memory_used_mb / 1024).toFixed(1)} GiB`}
            max={`${(stats.memory_total_mb / 1024).toFixed(1)} GiB`}
            label={t("dashboard.hostMemory")}
            fraction={memoryFraction}
            tone={memoryFraction > 0.9 ? "warn" : "accent"}
          />
          <StatCard
            compact
            shortLabel={t("dashboard.serversOnlineShort")}
            icon={<Icon.Package size={18} />}
            value={String(stats.servers_online)}
            max={String(servers.length)}
            label={t("dashboard.serversOnline")}
            fraction={onlineFraction}
          />
        </div>
      )}

      {creating && (
        <CreateServer
          onClose={() => setCreating(false)}
          onCreated={async () => {
            setCreating(false);
            await refresh();
          }}
        />
      )}

      <div class="grid gap-3 sm:gap-4">
        {servers.map((server) => {
          const actions = serverActionCapabilities(server.status);
          const cpuHostPercent =
            typeof server.metrics?.cpu_host_percent === "number" &&
            Number.isFinite(server.metrics.cpu_host_percent)
              ? Math.max(0, Math.min(100, server.metrics.cpu_host_percent))
              : null;
          return (
            <article
              key={server.id}
              data-testid="server-card"
              data-server-name={server.name}
              class="rounded-2xl border border-ink-700 bg-ink-850 p-3 sm:px-5 sm:py-4"
            >
              <div class="flex items-start gap-3">
                <div
                  class="grid size-9 shrink-0 place-items-center rounded-xl border border-ink-700 text-sm font-semibold text-fg sm:size-11 sm:text-base"
                  style={{ background: tileColour(server.id) }}
                  aria-hidden="true"
                >
                  {server.name.slice(0, 1).toUpperCase()}
                </div>
                <div class="min-w-0 flex-1">
                  <div class="flex items-start justify-between gap-2">
                    <div class="min-w-0">
                      <button
                        class="block max-w-full truncate text-left text-base font-semibold hover:text-accent"
                        onClick={() => onOpen(server.id)}
                      >
                        {server.name}
                      </button>
                      <div class="mt-1">
                        <StatusPill status={server.status} />
                      </div>
                    </div>
                    <Button
                      variant="ghost"
                      square
                      icon={<Icon.FolderOpen size={19} />}
                      aria-label={t("dashboard.manage")}
                      title={t("dashboard.manage")}
                      class="size-11 shrink-0 sm:h-auto sm:w-auto sm:px-3 sm:py-2"
                      onClick={() => onOpen(server.id)}
                    >
                      <span class="hidden sm:inline">{t("dashboard.manage")}</span>
                    </Button>
                  </div>

                  <p class="mt-2 truncate text-xs text-fg-muted">
                    {server.core} {server.version} · {t("createServer.port").toLowerCase()} {server.port} ·
                    Java {server.java_major} · {server.memory.max_mb} MiB
                  </p>
                  {server.uptime_secs !== null && (
                    <p class="mt-1 truncate text-xs text-fg-muted/80">
                      {t("dashboard.upFor", { duration: formatUptime(server.uptime_secs) })}
                      {server.metrics && (
                        <>
                          {" · "}
                          <span class="tabular-nums text-fg-muted">
                            {cpuHostPercent === null ? "—" : `${cpuHostPercent.toFixed(0)}%`} CPU ·{" "}
                            {server.metrics.memory_mb} MiB RSS
                          </span>
                        </>
                      )}
                    </p>
                  )}

                  <div class="mt-3 flex flex-wrap items-center gap-2">
                    <Button
                      variant="primary"
                      square
                      icon={<Icon.Play size={18} />}
                      aria-label={t("dashboard.start")}
                      title={t("dashboard.start")}
                      class="size-11 shrink-0 sm:h-auto sm:w-auto sm:px-4 sm:py-2"
                      disabled={!actions.start}
                      onClick={() => power(server.id, "start")}
                    >
                      <span class="hidden sm:inline">{t("dashboard.start")}</span>
                    </Button>
                    <Button
                      square
                      icon={<Icon.Restart size={19} />}
                      aria-label={t("dashboard.restart")}
                      title={t("dashboard.restart")}
                      class="size-11 shrink-0 sm:h-auto sm:w-auto sm:px-4 sm:py-2"
                      disabled={!actions.restart}
                      onClick={() => power(server.id, "restart")}
                    >
                      <span class="hidden sm:inline">{t("dashboard.restart")}</span>
                    </Button>
                    <Button
                      variant="danger"
                      square
                      icon={<Icon.Stop size={19} />}
                      aria-label={actions.cancel ? t("common.cancel") : t("dashboard.stop")}
                      title={actions.cancel ? t("common.cancel") : t("dashboard.stop")}
                      class="size-11 shrink-0 sm:h-auto sm:w-auto sm:px-4 sm:py-2"
                      disabled={!(actions.stop || actions.cancel)}
                      onClick={() => power(server.id, "stop")}
                    >
                      <span class="hidden sm:inline">
                        {actions.cancel ? t("common.cancel") : t("dashboard.stop")}
                      </span>
                    </Button>
                  </div>
                </div>
              </div>
            </article>
          );
        })}

        {servers.length === 0 && (
          <Card>
            <p class="text-center text-sm text-fg-muted">
              {t("dashboard.empty")}{" "}
              {user.admin ? t("dashboard.emptyAdmin") : t("dashboard.emptyUser")}
            </p>
          </Card>
        )}
      </div>
    </div>
  );
}

function CreateServer({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: () => void;
}) {
  const t = useT();
  const toast = useToast();
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
  const busyRef = useRef(false);

  useEffect(() => {
    api.providers().then(setProviders).catch(() => {});
  }, []);

  useEffect(() => {
    setVersions([]);
    api
      .versions(form.core)
      .then((list) => {
        setVersions(list);
        const version = list[0] ?? "";
        setForm((f) => ({
          ...f,
          version,
          // Minecraft version selection drives the compatible Java default.
          // The demo verifies this mapping instead of overriding it.
          java_major: version ? recommendedJavaForVersion(version) : f.java_major,
        }));
      })
      .catch((e) => toast.error(e.message));
  }, [form.core]);

  async function submit(event?: Event) {
    event?.preventDefault();
    if (busyRef.current || busy || !form.version) return;
    busyRef.current = true;
    setBusy(true);
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
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      busyRef.current = false;
      setBusy(false);
    }
  }

  const set = (patch: Partial<typeof form>) => setForm((f) => ({ ...f, ...patch }));

  return (
    <Modal
      title={t("dashboard.newServer")}
      onClose={onClose}
      width="lg"
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button
            type="submit"
            form="create-server-form"
            variant="primary"
            disabled={busy || !form.version}
            data-testid="server-create"
          >
            {busy ? t("common.creating") : t("createServer.submit")}
          </Button>
        </>
      }
    >
      <form id="create-server-form" onSubmit={submit} class="space-y-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <Field label={t("common.name")}>
            <Input
              value={form.name}
              placeholder="Survival"
              data-testid="server-name"
              onInput={(e) => set({ name: (e.target as HTMLInputElement).value })}
            />
          </Field>

          <Field label={t("createServer.port")}>
            <Input
              type="number"
              value={form.port}
              data-testid="server-port"
              onInput={(e) => set({ port: Number((e.target as HTMLInputElement).value) })}
            />
          </Field>

          <Field label={t("createServer.flavour")}>
            <Select
              value={form.core}
              data-testid="server-core"
              onChange={(e) => set({ core: (e.target as HTMLSelectElement).value })}
            >
              {providers.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.id}
                  {p.server ? "" : ` (${t("createServer.proxy")})`}
                </option>
              ))}
            </Select>
          </Field>

          <Field
            label={t("createServer.version")}
            hint={versions.length === 0 ? t("common.loading") : undefined}
          >
            <Select
              value={form.version}
              data-testid="server-version"
              onChange={(e) => {
                const version = (e.target as HTMLSelectElement).value;
                set({
                  version,
                  // Keep Java in sync with the selected Minecraft version.
                  java_major: recommendedJavaForVersion(version),
                });
              }}
            >
              {versions.map((v) => (
                <option key={v} value={v}>
                  {v}
                </option>
              ))}
            </Select>
          </Field>

          <Field label={t("createServer.javaVersion")} hint={t("createServer.javaHint")}>
            <Select
              value={String(form.java_major)}
              data-testid="server-java"
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
        </div>

        <label class="flex items-start gap-2.5 pt-1 text-sm">
          <input
            type="checkbox"
            checked={form.eula_accepted}
            data-testid="server-eula"
            onChange={(e) => set({ eula_accepted: (e.target as HTMLInputElement).checked })}
            class="mt-0.5 size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          <span class="text-fg-muted">
            {t("createServer.eulaPrefix")}{" "}
            <a
              href="https://aka.ms/MinecraftEULA"
              target="_blank"
              rel="noreferrer"
              class="text-accent underline"
            >
              {t("createServer.eulaLink")}
            </a>
            {t("createServer.eulaSuffix")}
          </span>
        </label>
      </form>
    </Modal>
  );
}
