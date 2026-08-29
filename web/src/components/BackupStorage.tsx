import { useEffect, useState } from "preact/hooks";
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

  if (!settings) return <Card>Loading backup settings…</Card>;

  return (
    <Card title="Backup Storage">
      <div class="space-y-4">
        <Field label="Backup provider">
          <Select value={provider} onChange={(e) => setProvider((e.target as HTMLSelectElement).value)}>
            <option value="local">Local</option>
            <option value="google_drive">Google Drive</option>
          </Select>
        </Field>

        {provider === "google_drive" && (
          <>
            <Field label="Google Drive Folder ID">
              <Input value={folderId} placeholder="1abc..." onInput={(e) => setFolderId((e.target as HTMLInputElement).value)} />
            </Field>
            <p class="text-xs text-fg-muted">
              Credentials: {settings.google_drive?.credentials_present ? "✓ Configured" : "Not configured"} — upload service-account JSON via API.
            </p>
            <Button variant="ghost" onClick={test} disabled={testing}>
              {testing ? "Testing…" : "Test connection"}
            </Button>
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
