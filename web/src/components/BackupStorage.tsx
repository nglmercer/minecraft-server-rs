import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useToast } from "./Toast";
import { Banner, Button, Card, Field, InfoTooltip, Input } from "./ui";
import * as Icon from "./icons";
import { useT } from "../i18n";

type OAuthStatus = { connected: boolean; configured: boolean };

export function BackupStorage() {
  const t = useT();
  const toast = useToast();
  const [settings, setSettings] = useState<any>(null);
  const [maxBackups, setMaxBackups] = useState(1);
  const [maxAge, setMaxAge] = useState<string>("");
  const [oauth, setOAuth] = useState<OAuthStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [oauthBusy, setOauthBusy] = useState(false);
  const pollRef = useRef<number | null>(null);

  async function load() {
    try {
      const [s, st] = await Promise.all([
        api.backupSettings(),
        api.googleOAuthStatus().catch(() => ({ connected: false, configured: false }) as OAuthStatus),
      ]);
      setSettings(s);
      setOAuth(st);
      setMaxBackups(s.retention.max_backups);
      setMaxAge(s.retention.max_age_days?.toString() ?? "");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    }
  }

  useEffect(() => {
    void load();
    return () => {
      if (pollRef.current) window.clearInterval(pollRef.current);
    };
  }, []);

  async function saveRetention() {
    setBusy(true);
    try {
      await api.updateBackupSettings({
        retention: {
          max_backups: maxBackups,
          max_age_days: maxAge ? Number(maxAge) : null,
        },
      });
      toast.success(t("backups.retentionSaved"));
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setBusy(false);
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
      const onFocus = async () => {
        try {
          const st = await api.googleOAuthStatus();
          setOAuth(st);
          if (st.connected) {
            if (pollRef.current) window.clearInterval(pollRef.current);
            pollRef.current = null;
            setOauthBusy(false);
            window.removeEventListener("focus", onFocus);
            await load();
          }
        } catch {}
      };
      window.addEventListener("focus", onFocus);
      setTimeout(() => window.removeEventListener("focus", onFocus), 65000);
    } catch (e) {
      const msg = e instanceof Error ? e.message : t("errors.generic");
      toast.error(msg);
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

  if (!settings) {
    return (
      <Card>
        <p class="text-sm text-fg-muted">{t("common.loading")}</p>
      </Card>
    );
  }

  const oauthConnected = !!oauth?.connected;
  const oauthConfigured = !!oauth?.configured;
  const redirectUri = `${location.origin}/api/settings/backups/google/oauth/callback`;

  return (
    <div class="space-y-6">
      {/* Global retention */}
      <Card
        title={
          <span class="flex items-center gap-2.5">
            <Icon.Archive size={16} class="text-accent" />
            <span>{t("backups.globalRetentionTitle")}</span>
            <span class="rounded-full bg-ink-700 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
              {t("backups.localDefault")}
            </span>
            <InfoTooltip text={t("backups.globalRetentionHelp")} />
          </span>
        }
      >
        <div class="space-y-4">
          <div class="grid gap-4 sm:grid-cols-2">
            <Field label={t("backups.maxBackupsLabel")}>
              <Input
                type="number"
                min={1}
                max={1000}
                value={maxBackups}
                onInput={(e) => setMaxBackups(Number((e.target as HTMLInputElement).value))}
              />
            </Field>
            <Field label={t("backups.maxAgeLabel")}>
              <Input
                value={maxAge}
                placeholder={t("backups.maxAgePlaceholder")}
                inputMode="numeric"
                onInput={(e) => setMaxAge((e.target as HTMLInputElement).value)}
              />
            </Field>
          </div>

          <div class="flex flex-wrap items-center justify-between gap-3 pt-1 border-t border-ink-700/60">
            <div class="flex items-center gap-2">
              <Button variant="primary" onClick={saveRetention} disabled={busy}>
                {busy ? t("common.saving") : t("backups.saveRetention")}
              </Button>
            </div>
            <div class="flex items-center gap-1.5 text-xs text-fg-muted">
              <span>{t("backups.globalRetentionNote")}</span>
              <InfoTooltip text={t("backups.retentionHelp")} />
            </div>
          </div>
        </div>
      </Card>

      {/* Google Drive */}
      <Card
        title={
          <span class="flex items-center gap-2.5">
            <Icon.Globe size={16} class="text-sky-400" />
            <span>{t("backups.googleDrive")}</span>
            <span class="rounded-full bg-ink-700 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wider text-fg-muted">
              {t("common.optional")}
            </span>
            <InfoTooltip text={t("backups.googleDriveHelp")} />
          </span>
        }
      >
        <div class="space-y-4">
          {!oauthConfigured && (
            <Banner kind="warn">
              <div class="space-y-2">
                <div class="flex items-center gap-2 font-semibold text-amber-200">
                  <Icon.Warning size={15} />
                  <span>{t("backups.oauthNotConfigured")}</span>
                </div>
                <p class="text-xs text-amber-100/80 leading-relaxed">{t("backups.oauthEnvHelp")}</p>
                <div class="text-xs text-amber-100/80">
                  <p>{t("backups.oauthRedirectUriHelp")}</p>
                  <code class="mt-1 block rounded bg-black/30 px-2 py-1 font-mono text-[11px] text-amber-100 break-all select-all">
                    {redirectUri}
                  </code>
                </div>
              </div>
            </Banner>
          )}

          {oauthConfigured && !oauthConnected && (
            <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-ink-700 bg-ink-900/60 p-4">
              <div class="flex items-center gap-2 text-sm font-medium text-fg">
                <span>{t("backups.notConnected")}</span>
                <InfoTooltip text={t("backups.notConnectedHelp")} />
              </div>
              <div class="flex flex-wrap items-center gap-2">
                <Button
                  variant="primary"
                  onClick={connectGoogle}
                  disabled={oauthBusy}
                  icon={<Icon.Link size={14} />}
                >
                  {oauthBusy ? t("backups.waitingConsent") : t("backups.connectGoogle")}
                </Button>
                <Button
                  variant="ghost"
                  onClick={test}
                  disabled={testing}
                  icon={<Icon.Refresh size={14} />}
                >
                  {testing ? t("common.loading") : t("backups.testConnection")}
                </Button>
              </div>
            </div>
          )}

          {oauthConfigured && oauthConnected && (
            <div class="flex flex-wrap items-center justify-between gap-3 rounded-xl border border-emerald-500/30 bg-emerald-500/10 p-4">
              <div class="flex items-center gap-2 text-sm font-medium text-emerald-200">
                <Icon.Check size={16} class="text-emerald-400" />
                <span>{t("backups.connected")}</span>
                <InfoTooltip text={t("backups.connectedHelp")} />
              </div>
              <div class="flex flex-wrap items-center gap-2">
                <Button
                  variant="ghost"
                  onClick={test}
                  disabled={testing}
                  icon={<Icon.Refresh size={14} />}
                >
                  {testing ? t("common.loading") : t("backups.testConnection")}
                </Button>
                <Button variant="danger" onClick={disconnectGoogle} disabled={oauthBusy}>
                  {t("backups.disconnect")}
                </Button>
              </div>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}
