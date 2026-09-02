import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useToast } from "./Toast";
import { Banner, Button, Card, Field, InfoTooltip, Input, Select } from "./ui";
import * as Icon from "./icons";
import { useT } from "../i18n";

export function ServerBackupSettings({ serverId }: { serverId: string }) {
  const t = useT();
  const toast = useToast();
  const [loading, setLoading] = useState(true);
  const [inheritGlobal, setInheritGlobal] = useState(true);
  const [provider, setProvider] = useState<"local" | "google_drive">("local");
  const [folderId, setFolderId] = useState("");
  const [maxBackups, setMaxBackups] = useState(1);
  const [maxAge, setMaxAge] = useState("");
  const [busy, setBusy] = useState(false);
  const [oauth, setOAuth] = useState<{ connected: boolean; configured: boolean } | null>(null);
  const [oauthBusy, setOauthBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const pollRef = useRef<number | null>(null);

  // For showing inherited values nicely
  const [globalRetention, setGlobalRetention] = useState<{ max_backups: number; max_age_days: number | null } | null>(null);

  async function load() {
    setLoading(true);
    try {
      const [s, st, g] = await Promise.all([
        api.serverBackupSettings(serverId),
        api.googleOAuthStatus().catch(() => ({ connected: false, configured: false })),
        api.backupSettings().catch(() => null),
      ]);
      setInheritGlobal(s.inherit_global);
      setOAuth(st);
      if (g) setGlobalRetention({ max_backups: g.retention.max_backups, max_age_days: g.retention.max_age_days ?? null });
      if (!s.inherit_global) {
        setProvider((s.provider as any) ?? "local");
        setMaxBackups(s.retention?.max_backups ?? 1);
        setMaxAge(s.retention?.max_age_days?.toString() ?? "");
        setFolderId(s.google_drive?.folder_id ?? "");
      } else {
        setProvider("local");
        setMaxBackups(g?.retention.max_backups ?? 1);
        setMaxAge(g?.retention.max_age_days?.toString() ?? "");
        setFolderId("");
      }
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setLoading(false);
    }
  }

  useEffect(() => {
    void load();
    return () => {
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, [serverId]);

  async function save() {
    setBusy(true);
    try {
      const body: any = inheritGlobal
        ? { inherit_global: true }
        : {
            inherit_global: false,
            provider,
            retention: {
              max_backups: maxBackups,
              max_age_days: maxAge ? Number(maxAge) : null,
            },
            google_drive: provider === "google_drive" ? { folder_id: folderId } : undefined,
          };
      await api.updateServerBackupSettings(serverId, body);
      toast.success(t("backups.perServerSaved"));
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setBusy(false);
    }
  }

  async function connectGoogle() {
    setOauthBusy(true);
    try {
      const redirectUri = `${location.origin}/api/settings/backups/google/oauth/callback`;
      const { url } = await api.startGoogleOAuth(redirectUri);
      window.open(url, "_blank", "width=620,height=720,popup=1");
      toast.success(t("backups.consentOpened"));
      let attempts = 0;
      if (pollRef.current) window.clearInterval(pollRef.current);
      pollRef.current = window.setInterval(async () => {
        attempts += 1;
        try {
          const st = await api.googleOAuthStatus();
          setOAuth(st);
          if (st.connected) {
            if (pollRef.current) window.clearInterval(pollRef.current);
            pollRef.current = null;
            setOauthBusy(false);
            toast.success(t("backups.driveConnected"));
            await load();
          } else if (attempts > 30) {
            if (pollRef.current) window.clearInterval(pollRef.current);
            pollRef.current = null;
            setOauthBusy(false);
          }
        } catch {}
      }, 2000);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
      setOauthBusy(false);
    }
  }

  async function disconnectGoogle() {
    setOauthBusy(true);
    try {
      await api.disconnectGoogleOAuth();
      toast.success(t("backups.driveDisconnected"));
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setOauthBusy(false);
    }
  }

  async function test() {
    setTesting(true);
    try {
      const res = await api.testBackupSettings();
      toast.success(res.message || t("backups.connectionOk"));
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setTesting(false);
    }
  }

  if (loading) {
    return (
      <Card>
        <p class="text-sm text-fg-muted">{t("common.loading")}</p>
      </Card>
    );
  }

  const isGDrive = provider === "google_drive";
  const oauthConnected = !!oauth?.connected;
  const oauthConfigured = !!oauth?.configured;

  return (
    <Card
      title={
        <span class="flex items-center gap-2.5">
          <Icon.Archive size={16} class="text-accent" />
          <span>{t("backups.perServerTitle")}</span>
          <span
            class={`rounded-full px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider ${
              inheritGlobal
                ? "bg-ink-700 text-fg-muted"
                : isGDrive
                  ? "bg-sky-500/20 text-sky-300"
                  : "bg-accent/20 text-accent"
            }`}
          >
            {inheritGlobal
              ? t("backups.inheritingLocal")
              : isGDrive
                ? t("backups.googleDrive")
                : t("backups.local")}
          </span>
          <InfoTooltip text={t("backups.whyLocalTooltip")} />
        </span>
      }
    >
      <div class="space-y-4">
        {/* Inherit toggle */}
        <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-ink-700 bg-ink-900/50 px-4 py-3">
          <label class="flex items-center gap-2.5 cursor-pointer text-sm font-medium text-fg">
            <input
              type="checkbox"
              checked={inheritGlobal}
              onChange={(e) => setInheritGlobal((e.target as HTMLInputElement).checked)}
              class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
            />
            <span>{t("backups.useGlobal")}</span>
          </label>
          <span class="text-xs text-fg-muted">
            {globalRetention
              ? t("backups.inheritsSummary", {
                  max: globalRetention.max_backups,
                  age: globalRetention.max_age_days
                    ? t("backups.maxDays", { days: globalRetention.max_age_days })
                    : t("backups.noAgeLimit"),
                })
              : t("backups.perServerHint")}
          </span>
        </div>

        {!inheritGlobal && (
          <div class="space-y-4 pt-1">
            <Field label={t("backups.providerLabel")}>
              <Select
                value={provider}
                onChange={(e) => setProvider((e.target as HTMLSelectElement).value as any)}
              >
                <option value="local">{t("backups.providerLocal")}</option>
                <option value="google_drive">{t("backups.providerDrive")}</option>
              </Select>
            </Field>

            {isGDrive && (
              <div class="space-y-3 rounded-xl border border-sky-500/20 bg-sky-500/[0.06] p-4">
                <Field
                  label={t("backups.driveFolderId")}
                  hint={t("backups.driveFolderHelp")}
                >
                  <Input
                    value={folderId}
                    placeholder={t("backups.driveFolderPlaceholder")}
                    onInput={(e) => setFolderId((e.target as HTMLInputElement).value)}
                  />
                </Field>

                <div class="space-y-2">
                  {!oauthConfigured && (
                    <Banner kind="warn">
                      <div class="flex items-center gap-2 text-xs">
                        <Icon.Warning size={14} />
                        <span>
                          {t("backups.oauthNotConfigured")} — {t("backups.connectFromGlobal")}
                        </span>
                      </div>
                    </Banner>
                  )}

                  {oauthConfigured && !oauthConnected && (
                    <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-ink-700 bg-ink-900 px-3 py-2.5">
                      <div class="flex items-center gap-2 text-xs text-fg-muted">
                        <span>{t("backups.notConnected")}</span>
                        <InfoTooltip text={t("backups.notConnectedHelp")} />
                      </div>
                      <Button
                        variant="primary"
                        onClick={connectGoogle}
                        disabled={oauthBusy}
                        class="!px-3 !py-1 !text-xs"
                      >
                        {oauthBusy ? t("backups.waitingConsent") : t("backups.connectGoogle")}
                      </Button>
                    </div>
                  )}

                  {oauthConnected && (
                    <div class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-3 py-2.5">
                      <div class="flex items-center gap-2 text-xs text-emerald-200">
                        <Icon.Check size={14} class="text-emerald-400" />
                        <span>{t("backups.connected")}</span>
                        <InfoTooltip text={t("backups.driveServerTokenHelp")} />
                      </div>
                      <div class="flex items-center gap-2">
                        <Button
                          variant="ghost"
                          onClick={test}
                          disabled={testing}
                          class="!px-2.5 !py-1 !text-xs"
                        >
                          {testing ? t("common.loading") : t("backups.testConnection")}
                        </Button>
                        <Button
                          variant="danger"
                          onClick={disconnectGoogle}
                          disabled={oauthBusy}
                          class="!px-2.5 !py-1 !text-xs"
                        >
                          {t("backups.disconnect")}
                        </Button>
                      </div>
                    </div>
                  )}
                </div>
              </div>
            )}

            <div class="grid gap-4 sm:grid-cols-2">
              <Field label={t("backups.maxBackupsLabel")}>
                <Input
                  type="number"
                  min={1}
                  value={maxBackups}
                  onInput={(e) => setMaxBackups(Number((e.target as HTMLInputElement).value))}
                />
              </Field>
              <Field label={t("backups.maxAgeLabel")}>
                <Input
                  value={maxAge}
                  placeholder={t("backups.maxAgePlaceholder")}
                  onInput={(e) => setMaxAge((e.target as HTMLInputElement).value)}
                />
              </Field>
            </div>
          </div>
        )}

        <div class="flex flex-wrap items-center justify-between gap-3 border-t border-ink-700/60 pt-3">
          <Button variant="primary" onClick={save} disabled={busy}>
            {busy ? t("common.saving") : t("common.save")}
          </Button>
          {!inheritGlobal && (
            <div class="flex items-center gap-1.5 text-xs text-fg-muted">
              <span>{t("backups.perServerHint")}</span>
              <InfoTooltip text={t("backups.retentionHelp")} />
            </div>
          )}
        </div>
      </div>
    </Card>
  );
}
