import { useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useToast } from "./Toast";
import { Button, Card, Field, Input, Select } from "./ui";

export function BackupStorage() {
  const toast = useToast();
  const [settings, setSettings] = useState<any>(null);
  const [provider, setProvider] = useState("local");
  const [maxBackups, setMaxBackups] = useState(1);
  const [maxAge, setMaxAge] = useState<string>("");
  const [folderId, setFolderId] = useState("");
  const [busy, setBusy] = useState(false);
  const [testing, setTesting] = useState(false);
  const [oauthBusy, setOauthBusy] = useState(false);
  const pollRef = useRef<number | null>(null);

  async function load() {
    try {
      const s = await api.backupSettings();
      setSettings(s);
      setProvider(s.provider);
      setMaxBackups(s.retention.max_backups);
      setMaxAge(s.retention.max_age_days?.toString() ?? "");
      setFolderId(s.google_drive?.folder_id ?? "");
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

  async function save() {
    setBusy(true);
    try {
      const body: any = {
        provider,
        retention: {
          max_backups: maxBackups,
          max_age_days: maxAge ? Number(maxAge) : null,
        },
      };
      if (provider === "google_drive") {
        body.google_drive = { folder_id: folderId };
      }
      await api.updateBackupSettings(body);
      toast.success("Backup settings saved");
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
      // Poll status until connected (60s) or window closed
      let attempts = 0;
      if (pollRef.current) window.clearInterval(pollRef.current);
      pollRef.current = window.setInterval(async () => {
        attempts += 1;
        try {
          const st = await api.googleOAuthStatus();
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
      // Also stop on focus return
      const onFocus = async () => {
        try {
          const s = await api.backupSettings();
          if (s.google_drive?.oauth_connected) {
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

  if (!settings) return <Card>Loading backup settings…</Card>;

  const gd = settings.google_drive;
  const oauthConnected = !!gd?.oauth_connected;
  const oauthConfigured = gd?.oauth_configured ?? true; // assume true when gd null but client exists check in status
  const credPresent = !!gd?.credentials_present;
  const isGDrive = provider === "google_drive";

  return (
    <Card title="Backup Storage">
      <div class="space-y-4">
        <Field label="Backup provider">
          <Select value={provider} onChange={(e) => setProvider((e.target as HTMLSelectElement).value)}>
            <option value="local">Local</option>
            <option value="google_drive">Google Drive</option>
          </Select>
        </Field>

        {isGDrive && (
          <>
            <Field label="Google Drive Folder ID">
              <Input value={folderId} placeholder="1abc... (leave empty for My Drive root)" onInput={(e) => setFolderId((e.target as HTMLInputElement).value)} />
            </Field>
            <div class="rounded-lg border border-ink-700 bg-ink-900 px-3 py-3 text-xs leading-relaxed">
              {!oauthConfigured && (
                <p class="text-amber-300">OAuth not configured on server: set <code class="font-mono">MCPANEL_GOOGLE_CLIENT_ID</code> and <code class="font-mono">MCPANEL_GOOGLE_CLIENT_SECRET</code> then restart the panel. Redirect URI: <code class="font-mono">{location.origin}/api/settings/backups/google/oauth/callback</code></p>
              )}
              {oauthConfigured && !oauthConnected && (
                <>
                  <p class="text-fg-muted">Not connected — click below to sign in with Google. Your drive files stay private; the panel only stores a refresh token.</p>
                  <Button variant="primary" onClick={connectGoogle} disabled={oauthBusy} icon={<span aria-hidden>↗</span>} class="mt-2">
                    {oauthBusy ? "Waiting for consent…" : "Connect Google Drive"}
                  </Button>
                </>
              )}
              {oauthConnected && (
                <>
                  <p class="text-emerald-300">✓ Connected to Google Drive</p>
                  {credPresent && <p class="text-fg-muted">Service-account credentials also present (OAuth takes precedence).</p>}
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
              {!oauthConnected && oauthConfigured && !oauthConnected && (
                <div class="mt-2">
                  <Button variant="ghost" onClick={test} disabled={testing}>
                    {testing ? "Testing…" : "Test connection"}
                  </Button>
                </div>
              )}
            </div>
          </>
        )}

        <Field label="Maximum backups">
          <Input type="number" value={maxBackups} onInput={(e) => setMaxBackups(Number((e.target as HTMLInputElement).value))} />
        </Field>
        <Field label="Maximum age (days) — empty = disabled">
          <Input value={maxAge} placeholder="Disabled" onInput={(e) => setMaxAge((e.target as HTMLInputElement).value)} />
        </Field>
        <p class="text-xs text-fg-muted">Whichever limit is reached first applies. The newest successful backup is always protected while retention runs.</p>

        <Button variant="primary" onClick={save} disabled={busy}>
          {busy ? "Saving…" : "Save"}
        </Button>
      </div>
    </Card>
  );
}
