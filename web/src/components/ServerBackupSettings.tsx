import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useToast } from "./Toast";
import { Button, Card, Field, Input, Select } from "./ui";
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
      toast.error(e instanceof Error ? e.message : "failed to load server backup settings");
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
      toast.success("Server backup settings saved");
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "save failed");
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
      toast.success("Opened Google consent — complete in the new window");
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
            toast.success("Google Drive connected");
            await load();
          } else if (attempts > 30) {
            if (pollRef.current) window.clearInterval(pollRef.current);
            pollRef.current = null;
            setOauthBusy(false);
          }
        } catch {}
      }, 2000);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "failed to start OAuth");
      setOauthBusy(false);
    }
  }

  async function disconnectGoogle() {
    setOauthBusy(true);
    try {
      await api.disconnectGoogleOAuth();
      toast.success("Google Drive disconnected");
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "disconnect failed");
    } finally {
      setOauthBusy(false);
    }
  }

  async function test() {
    setTesting(true);
    try {
      const res = await api.testBackupSettings();
      toast.success(res.message || "Connection ok");
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "test failed");
    } finally {
      setTesting(false);
    }
  }

  if (loading) return <Card>Loading…</Card>;

  const isGDrive = provider === "google_drive";
  const oauthConnected = !!oauth?.connected;
  const oauthConfigured = !!oauth?.configured;

  return (
    <Card
      title={
        <span class="flex items-center gap-2">
          <span class="grid size-7 place-items-center rounded-lg bg-ink-700 text-fg-muted">
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path d="M14 2H7a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" />
              <path d="M14 2v6h6M10 13H8M16 17H8M13 13h1" />
            </svg>
          </span>
          {t("backups.perServerTitle")}
          <span class={`rounded-full px-2 py-0.5 text-[10px] font-bold uppercase tracking-widest ${inheritGlobal ? "bg-ink-700 text-fg-muted" : isGDrive ? "bg-sky-500/20 text-sky-300" : "bg-accent/20 text-accent"}`}>
            {inheritGlobal ? "Inheriting • Local" : isGDrive ? "Google Drive" : "Local"}
          </span>
        </span>
      }
    >
      <div class="space-y-5">
        {/* Inherit toggle */}
        <label class="flex items-start gap-3 rounded-lg border border-ink-700 bg-ink-900/50 px-4 py-3">
          <input
            type="checkbox"
            checked={inheritGlobal}
            onChange={(e) => setInheritGlobal((e.target as HTMLInputElement).checked)}
            class="mt-0.5 size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          <span class="min-w-0 flex-1">
            <span class="block text-sm font-medium text-fg">{t("backups.useGlobal")} — default is local</span>
            <span class="mt-1 block text-xs leading-relaxed text-fg-muted">
              {globalRetention
                ? `Inherits: local • keeps ${globalRetention.max_backups} backup(s)${globalRetention.max_age_days ? ` • max ${globalRetention.max_age_days} days` : " • no age limit"}`
                : t("backups.perServerHint")}
            </span>
          </span>
        </label>

        {!inheritGlobal && (
          <div class="space-y-4">
            {/* Provider */}
            <div class="grid gap-3 sm:grid-cols-[1fr_auto] items-end">
              <Field label={t("backups.providerLabel")}>
                <Select value={provider} onChange={(e) => setProvider((e.target as HTMLSelectElement).value as any)}>
                  <option value="local">Local — on this machine</option>
                  <option value="google_drive">Google Drive — cloud</option>
                </Select>
              </Field>
              <div class="hidden sm:block pb-1 text-xs text-fg-muted">
                {isGDrive ? "Cloud" : "Local"} storage
              </div>
            </div>

            {isGDrive && (
              <div class="space-y-3 rounded-xl border border-sky-500/20 bg-sky-500/[0.06] p-4">
                <Field label="Google Drive Folder ID">
                  <Input value={folderId} placeholder="1abc… (or empty for My Drive root)" onInput={(e) => setFolderId((e.target as HTMLInputElement).value)} />
                </Field>
                <p class="text-xs leading-relaxed text-fg-muted">Same folder is used for all backups of this server. You can change it without losing existing backups — they stay reachable at their original location.</p>

                <div class="rounded-lg border border-ink-700 bg-ink-900 px-3 py-3 text-xs leading-relaxed">
                  {!oauthConfigured && (
                    <p class="text-amber-300">
                      OAuth not configured on server — set <code class="font-mono">MCPANEL_GOOGLE_CLIENT_ID</code> / <code class="font-mono">MCPANEL_GOOGLE_CLIENT_SECRET</code> then restart. Connect from the global Backups page first.
                    </p>
                  )}
                  {oauthConfigured && !oauthConnected && (
                    <>
                      <p class="text-fg-muted">Not connected — sign in to allow this server to write to Drive.</p>
                      <Button variant="primary" onClick={connectGoogle} disabled={oauthBusy} class="mt-2">
                        {oauthBusy ? "Waiting…" : "Connect Google Drive"}
                      </Button>
                    </>
                  )}
                  {oauthConnected && (
                    <>
                      <p class="text-emerald-300">✓ Drive connected — this server will use the global OAuth token.</p>
                      <div class="mt-2 flex flex-wrap gap-2">
                        <Button variant="ghost" onClick={test} disabled={testing}>
                          {testing ? "Testing…" : "Test connection"}
                        </Button>
                        <Button variant="danger" onClick={disconnectGoogle} disabled={oauthBusy}>
                          Disconnect
                        </Button>
                      </div>
                    </>
                  )}
                </div>
              </div>
            )}

            <div class="grid gap-4 sm:grid-cols-2">
              <Field label={t("backups.maxBackupsLabel")}>
                <Input type="number" min={1} value={maxBackups} onInput={(e) => setMaxBackups(Number((e.target as HTMLInputElement).value))} />
              </Field>
              <Field label={t("backups.maxAgeLabel")}>
                <Input value={maxAge} placeholder="Disabled" onInput={(e) => setMaxAge((e.target as HTMLInputElement).value)} />
              </Field>
            </div>
            <p class="text-xs leading-relaxed text-fg-muted">
              This overrides global retention only for this server. Newest backup is always protected from pruning.
            </p>
          </div>
        )}

        {inheritGlobal && (
          <div class="rounded-lg border border-ink-700 bg-ink-850 px-4 py-3 text-xs leading-relaxed text-fg-muted">
            <span class="font-medium text-fg">Why local by default?</span> Keeps restores instant and works offline. Switch off “Use global settings” above to store this server in Google Drive instead, or to keep more/fewer backups per world.
          </div>
        )}

        <div class="flex flex-wrap items-center gap-3 border-t border-ink-700 pt-4">
          <Button variant="primary" onClick={save} disabled={busy}>
            {busy ? t("common.saving") : t("common.save")}
          </Button>
          {!inheritGlobal && isGDrive && !folderId.trim() && (
            <span class="text-xs text-amber-300">Tip: leave folder empty for Drive root, or paste a folder ID from Drive URL.</span>
          )}
        </div>
      </div>
    </Card>
  );
}
