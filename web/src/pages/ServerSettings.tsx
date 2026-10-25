import { lazy, Suspense } from "preact/compat";
import { Loading } from "../components/Loading";
import { useState } from "preact/hooks";
import { api } from "../api";
import { Compatibility } from "../components/Compatibility";
import {
  Banner,
  Button,
  Card,
  Field,
  Input,
  Select,
} from "../components/ui";
import * as Icon from "../components/icons";
import { useDialogs } from "../components/Modal";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import type {
  PlayitAttachDisposition,
  Server,
  ServerPlayitView,
  User,
} from "../types";


const ChangeServerVersion = lazy(() => import("../components/ChangeServerVersion").then(m => ({ default: m.ChangeServerVersion })));

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
      {changingVersion && <Suspense fallback={<Loading />}><ChangeServerVersion server={server} onClose={() => setChangingVersion(false)} onSaved={onChanged} /></Suspense>}
    </Card>
  );
}

export function Settings({
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
