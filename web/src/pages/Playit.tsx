import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import {
  Actions,
  Banner,
  Button,
  Card,
  Empty,
  Field,
  Select,
} from "../components/ui";
import * as Icon from "../components/icons";
import { useDialogs } from "../components/Modal";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import type {
  PlayitAccount,
  PlayitAccountStatus,
  PlayitConnectionState,
  PlayitStatus,
  PlayitTunnel,
  Server,
} from "../types";

export function Playit() {
  const t = useT();
  const toast = useToast();
  const dialogs = useDialogs();

  const [status, setStatus] = useState<PlayitStatus | null>(null);
  const [account, setAccount] = useState<PlayitAccount | null>(null);
  const [tunnels, setTunnels] = useState<PlayitTunnel[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [claimUrl, setClaimUrl] = useState<string | null>(null);
  const [selectedServer, setSelectedServer] = useState("");
  const [failed, setFailed] = useState<string | null>(null);
  const [accountError, setAccountError] = useState<string | null>(null);
  const [tunnelError, setTunnelError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);

  async function refresh() {
    setLoading(true);
    const results = await Promise.allSettled([
      api.playitStatus(),
      api.playitAccount(),
      api.playitTunnels(),
      api.servers(),
    ]);

    const [statusResult, accountResult, tunnelResult, serverResult] = results;

    if (statusResult.status === "fulfilled") {
      setStatus(statusResult.value);
      setFailed(null);
    } else {
      setFailed(errorText(statusResult.reason, t("errors.loadPlayit")));
    }

    if (accountResult.status === "fulfilled") {
      setAccount(accountResult.value);
      setAccountError(null);
    } else {
      setAccountError(errorText(accountResult.reason, t("errors.loadPlayitAccount")));
    }

    if (tunnelResult.status === "fulfilled") {
      setTunnels(tunnelResult.value);
      setTunnelError(null);
    } else {
      setTunnelError(errorText(tunnelResult.reason, t("errors.loadPlayitTunnels")));
    }

    if (serverResult.status === "fulfilled") setServers(serverResult.value);
    else setFailed(errorText(serverResult.reason, t("errors.loadServers")));

    setLoading(false);
  }

  useEffect(() => {
    void refresh();
    const timer = setInterval(refresh, 5000);
    return () => clearInterval(timer);
  }, []);

  async function claim() {
    setBusy(true);
    try {
      const result = await api.playitClaim();
      setClaimUrl(result.claim_url);
      toast.success(t("playit.claimStarted"));
      await refresh();
    } catch (e) {
      toast.error(errorText(e, t("errors.playitAction")));
    } finally {
      setBusy(false);
    }
  }

  async function attach() {
    if (!selectedServer) return;
    setBusy(true);
    try {
      await api.attachPlayit(selectedServer);
      toast.success(t("playit.tunnelCreated"));
      setSelectedServer("");
      await refresh();
    } catch (e) {
      toast.error(errorText(e, t("errors.playitAction")));
    } finally {
      setBusy(false);
    }
  }

  async function remove(tunnel: PlayitTunnel) {
    const server = servers.find((candidate) => candidate.playit?.tunnel_id === tunnel.id);
    const name = server?.name ?? tunnel.name ?? tunnel.id;
    const confirmed = await dialogs.confirm({
      title: t("playit.deleteTunnelTitle", { name }),
      body: server ? t("playit.detachTunnelBody") : t("playit.deleteTunnelBody"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    setBusy(true);
    try {
      if (server) await api.detachPlayit(server.id);
      else await api.deletePlayitTunnel(tunnel.id);
      toast.success(t("playit.tunnelDeleted"));
      await refresh();
    } catch (e) {
      toast.error(errorText(e, t("errors.playitAction")));
    } finally {
      setBusy(false);
    }
  }

  async function copyAddress(address: string) {
    if (!navigator.clipboard) {
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

  const availableServers = servers.filter((server) => !server.playit);
  const loginUrl = safeExternalUrl(account?.login_link);
  const activeClaimUrl = claimUrl ?? account?.claim_url;
  const safeClaimUrl = safeExternalUrl(activeClaimUrl);

  return (
    <div class="mx-auto w-full max-w-6xl space-y-6 px-4 py-8 sm:px-6">
      <header class="flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 class="text-2xl font-semibold">{t("playit.title")}</h1>
          <p class="text-sm text-fg-muted">{t("playit.subtitle")}</p>
        </div>
        <Button
          variant="ghost"
          icon={<Icon.Refresh size={15} />}
          disabled={loading}
          onClick={() => void refresh()}
        >
          {t("common.refresh")}
        </Button>
      </header>

      {failed && <Banner kind="error">{failed}</Banner>}

      <Card title={t("playit.connectionSection")}>
        <div class="grid gap-4 sm:grid-cols-3">
          <Detail
            label={t("playit.connection")}
            value={status ? stateLabel(status.status, t) : t("common.loading")}
            tone={statusTone(status?.status)}
          />
          <Detail label={t("playit.version")} value={status?.version ?? t("common.none")} />
          <Detail
            label={t("playit.account")}
            value={account ? accountLabel(account.status, t) : t("common.none")}
          />
        </div>

        {status?.message && <p class="mt-4 text-sm text-fg-muted">{status.message}</p>}
        {account?.agent_id && (
          <p class="mt-3 text-xs text-fg-muted">
            {t("playit.agentId")}: <span class="font-mono text-fg">{account.agent_id}</span>
          </p>
        )}
        {accountError && <Banner kind="error">{accountError}</Banner>}

        <Actions>
          {status?.status === "needs_claim" && (
            <Button variant="primary" disabled={busy} onClick={() => void claim()}>
              {busy ? t("playit.startingClaim") : t("playit.connect")}
            </Button>
          )}
          {loginUrl && (
            <a
              class="inline-flex items-center gap-2 rounded-full bg-ink-700 px-4 py-2 text-sm font-medium text-fg hover:bg-ink-600"
              href={loginUrl}
              target="_blank"
              rel="noopener noreferrer"
            >
              {t("playit.openAccount")}
            </a>
          )}
        </Actions>

        {activeClaimUrl && (
          <div class="mt-4 space-y-2">
            <Banner kind="info">
              {t("playit.claimInstructions")} {" "}
              {safeClaimUrl ? (
                <a
                  class="font-medium text-accent underline"
                  href={safeClaimUrl}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {t("playit.openClaim")}
                </a>
              ) : (
                t("playit.claimLinkUnavailable")
              )}
            </Banner>
          </div>
        )}
      </Card>

      <Card title={t("playit.serverSection")}>
        <p class="mb-4 text-sm text-fg-muted">{t("playit.serverExplain")}</p>
        <div class="flex flex-wrap items-end gap-3">
          <div class="min-w-60 flex-1">
            <Field label={t("playit.server")}>
              <Select
                value={selectedServer}
                onChange={(event) => setSelectedServer((event.target as HTMLSelectElement).value)}
              >
                <option value="">{t("playit.chooseServer")}</option>
                {availableServers.map((server) => (
                  <option key={server.id} value={server.id}>
                    {server.name} · :{server.port}
                  </option>
                ))}
              </Select>
            </Field>
          </div>
          <Button
            variant="primary"
            icon={<Icon.Plus size={15} />}
            disabled={busy || !selectedServer || status?.status !== "connected"}
            onClick={() => void attach()}
          >
            {t("playit.createServerTunnel")}
          </Button>
        </div>
        {status?.status !== "connected" && (
          <p class="mt-3 text-xs text-fg-muted">{t("playit.connectBeforeTunnel")}</p>
        )}
      </Card>

      <Card title={t("playit.tunnelsSection")}>
        {tunnelError && <Banner kind="error">{tunnelError}</Banner>}
        {tunnels.length === 0 ? (
          <Empty>{t("playit.noTunnels")}</Empty>
        ) : (
          <div class="divide-y divide-ink-700">
            {tunnels.map((tunnel) => {
              const server = servers.find((candidate) => candidate.playit?.tunnel_id === tunnel.id);
              return (
                <article key={tunnel.id} class="flex flex-wrap items-center justify-between gap-4 py-4 first:pt-0 last:pb-0">
                  <div class="min-w-0 space-y-1">
                    <p class="font-medium">{server?.name ?? tunnel.name ?? t("playit.unmanaged")}</p>
                    <p class="text-xs text-fg-muted">
                      {tunnel.protocol.toUpperCase()} · {tunnel.destination}
                      {tunnel.disabled && ` · ${t("playit.disabled")}`}
                    </p>
                    {tunnel.disabled_reason && (
                      <p class="text-xs text-amber-300">{tunnel.disabled_reason}</p>
                    )}
                  </div>
                  <div class="flex flex-wrap items-center gap-2">
                    <Button
                      variant="ghost"
                      icon={<Icon.Copy size={15} />}
                      disabled={!tunnel.display_address}
                      onClick={() => void copyAddress(tunnel.display_address)}
                    >
                      {tunnel.display_address || t("common.none")}
                    </Button>
                    <Button
                      variant="danger"
                      icon={<Icon.Trash size={15} />}
                      disabled={busy}
                      onClick={() => void remove(tunnel)}
                    >
                      {t("common.delete")}
                    </Button>
                  </div>
                </article>
              );
            })}
          </div>
        )}
      </Card>
    </div>
  );
}

function Detail({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone?: "good" | "warn" | "bad";
}) {
  const colours = {
    good: "text-accent",
    warn: "text-amber-300",
    bad: "text-red-300",
  };
  return (
    <div class="rounded-lg border border-ink-700 bg-ink-900/60 px-4 py-3">
      <p class="text-xs uppercase tracking-wider text-fg-muted">{label}</p>
      <p class={`mt-1 font-medium ${tone ? colours[tone] : "text-fg"}`}>{value}</p>
    </div>
  );
}

function stateLabel(
  state: PlayitConnectionState,
  t: ReturnType<typeof useT>,
): string {
  return t(`playit.states.${state}` as "playit.states.connected");
}

function accountLabel(
  state: PlayitAccountStatus,
  t: ReturnType<typeof useT>,
): string {
  return t(`playit.accountStates.${state}` as "playit.accountStates.unknown");
}

function statusTone(state: PlayitConnectionState | undefined): "good" | "warn" | "bad" | undefined {
  if (state === "connected") return "good";
  if (state === "needs_claim" || state === "starting" || state === "stopping") return "warn";
  if (state) return "bad";
  return undefined;
}

function errorText(error: unknown, fallback: string): string {
  return error instanceof Error && error.message ? error.message : fallback;
}

function safeExternalUrl(value: string | null | undefined): string | null {
  if (!value) return null;
  try {
    const url = new URL(value);
    return url.protocol === "https:" ? url.href : null;
  } catch {
    return null;
  }
}
