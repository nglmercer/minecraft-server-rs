import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useToast } from "./Toast";
import { Button, Card, Field, Input } from "./ui";

type OAuthStatus = { connected: boolean; configured: boolean };

export function BackupStorage() {
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
      toast.error(e instanceof Error ? e.message : "failed to load backup settings");
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
      toast.success("Retention settings saved");
      await load();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : "failed to save");
    } finally {
      setBusy(false);
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

  async function connectGoogle() {
    setOauthBusy(true);
    try {
      const redirectUri = `${location.origin}/api/settings/backups/google/oauth/callback`;
      const { url } = await api.startGoogleOAuth(redirectUri);
      window.open(url, "_blank", "width=620,height=720,popup=1");
      toast.success("Opened Google consent — complete sign-in in the new window");
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
      const msg = e instanceof Error ? e.message : "failed to start OAuth";
      // Surface the server's helpful config message instead of generic 500
      toast.error(msg);
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

  if (!settings) return <Card>Loading backup settings…</Card>;

  const oauthConnected = !!oauth?.connected;
  const oauthConfigured = !!oauth?.configured;
  const redirectUri = `${location.origin}/api/settings/backups/google/oauth/callback`;

  return (
    <div class="space-y-5">
      {/* Global retention — always local */}
      <Card
        title={
          <span class="flex items-center gap-2">
            <span class="grid size-7 place-items-center rounded-lg bg-accent/15 text-accent">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M3 7h18M3 12h18M3 17h18" />
              </svg>
            </span>
            Global Retention
            <span class="rounded-full bg-ink-700 px-2 py-0.5 text-[10px] font-bold uppercase tracking-widest text-fg-muted">Local • Default</span>
          </span>
        }
      >
        <div class="space-y-4">
          <div class="rounded-lg border border-ink-700 bg-ink-900/60 px-4 py-3">
            <p class="text-sm font-medium text-fg">Backups are local by default</p>
            <p class="mt-1 text-xs leading-relaxed text-fg-muted">
              New servers store backups on this machine — fast restores, no external account needed.
              To use Google Drive, configure it per server below. Retention below applies to all servers that inherit global settings.
            </p>
          </div>

          <div class="grid gap-4 sm:grid-cols-2">
            <Field label="Maximum backups to keep">
              <Input
                type="number"
                min={1}
                max={1000}
                value={maxBackups}
                onInput={(e) => setMaxBackups(Number((e.target as HTMLInputElement).value))}
              />
            </Field>
            <Field label="Maximum age (days) — empty = keep forever">
              <Input
                value={maxAge}
                placeholder="Disabled"
                inputMode="numeric"
                onInput={(e) => setMaxAge((e.target as HTMLInputElement).value)}
              />
            </Field>
          </div>
          <p class="text-xs leading-relaxed text-fg-muted">
            Whichever limit hits first trims oldest backups. The newest successful backup is always kept, even when limits would otherwise delete it.
          </p>
          <div class="flex flex-wrap items-center gap-2">
            <Button variant="primary" onClick={saveRetention} disabled={busy}>
              {busy ? "Saving…" : "Save retention"}
            </Button>
            <span class="text-xs text-fg-muted">Applies to all servers using global settings.</span>
          </div>
        </div>
      </Card>

      {/* Google Drive — optional, global OAuth shared by per-server configs */}
      <Card
        title={
          <span class="flex items-center gap-2">
            <span class="grid size-7 place-items-center rounded-lg bg-sky-500/15 text-sky-300">
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M12 3l7 12H5L12 3z" />
                <path d="M5 15l7 6 7-6" />
              </svg>
            </span>
            Google Drive
            <span class="rounded-full bg-ink-700 px-2 py-0.5 text-[10px] font-semibold uppercase tracking-widest text-fg-muted">Optional</span>
          </span>
        }
      >
        <div class="space-y-3">
          <p class="text-xs leading-relaxed text-fg-muted">
            Connect Google Drive once here. Any server can then be switched to Drive in its own backup settings — you only pick the Drive folder per server.
          </p>

          {!oauthConfigured && (
            <div class="rounded-lg border border-amber-500/40 bg-amber-500/10 px-4 py-3 text-xs leading-relaxed">
              <p class="font-semibold text-amber-200">OAuth not configured on server</p>
              <p class="mt-1 text-amber-100/80">
                Set <code class="rounded bg-black/30 px-1 py-0.5 font-mono text-amber-100">MCPANEL_GOOGLE_CLIENT_ID</code> and{" "}
                <code class="rounded bg-black/30 px-1 py-0.5 font-mono text-amber-100">MCPANEL_GOOGLE_CLIENT_SECRET</code> then restart the panel.
              </p>
              <p class="mt-2 text-amber-100/80">
                In Google Cloud Console → OAuth client, add redirect URI:
                <br />
                <code class="mt-1 inline-block rounded bg-black/30 px-2 py-1 font-mono text-[11px] text-amber-100 break-all">{redirectUri}</code>
              </p>
            </div>
          )}

          {oauthConfigured && !oauthConnected && (
            <div class="rounded-lg border border-ink-700 bg-ink-900 px-4 py-4">
              <p class="text-sm font-medium text-fg">Not connected</p>
              <p class="mt-1 text-xs leading-relaxed text-fg-muted">
                Click below to sign in with Google. Your Drive stays private — the panel only stores a refresh token. No service-account JSON needed.
              </p>
              <div class="mt-3 flex flex-wrap gap-2">
                <Button variant="primary" onClick={connectGoogle} disabled={oauthBusy} icon={<span aria-hidden>↗</span>}>
                  {oauthBusy ? "Waiting for consent…" : "Connect Google Drive"}
                </Button>
                <Button variant="ghost" onClick={test} disabled={testing}>
                  {testing ? "Testing…" : "Test connection"}
                </Button>
              </div>
            </div>
          )}

          {oauthConfigured && oauthConnected && (
            <div class="rounded-lg border border-emerald-500/30 bg-emerald-500/10 px-4 py-4">
              <p class="flex items-center gap-2 text-sm font-semibold text-emerald-200">
                <span class="grid size-5 place-items-center rounded-full bg-emerald-500 text-white">
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3">
                    <path d="M5 13l4 4L19 7" />
                  </svg>
                </span>
                Connected to Google Drive
              </p>
              <p class="mt-1 text-xs leading-relaxed text-emerald-100/70">
                Any server set to Google Drive will upload here using this account. Folder is chosen per server.
              </p>
              <div class="mt-3 flex flex-wrap gap-2">
                <Button variant="ghost" onClick={test} disabled={testing}>
                  {testing ? "Testing…" : "Test connection"}
                </Button>
                <Button variant="danger" onClick={disconnectGoogle} disabled={oauthBusy}>
                  Disconnect
                </Button>
              </div>
            </div>
          )}
        </div>
      </Card>
    </div>
  );
}
