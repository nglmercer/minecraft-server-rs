import { useEffect, useState } from "preact/hooks";
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

  async function load() {
    setLoading(true);
    try {
      const s = await api.serverBackupSettings(serverId);
      setInheritGlobal(s.inherit_global);
      if (!s.inherit_global) {
        setProvider((s.provider as any) ?? "local");
        setMaxBackups(s.retention?.max_backups ?? 1);
        setMaxAge(s.retention?.max_age_days?.toString() ?? "");
        setFolderId(s.google_drive?.folder_id ?? "");
      } else {
        // Reset to defaults for local when inherited (UI shows local preview)
        setProvider("local");
        setMaxBackups(1);
        setMaxAge("");
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

  if (loading) return <Card>Loading…</Card>;

  return (
    <Card title={t("backups.perServerTitle")}>
      <div class="space-y-4">
        <label class="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={inheritGlobal}
            onChange={(e) => setInheritGlobal((e.target as HTMLInputElement).checked)}
            class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          <span>{t("backups.useGlobal")} — default is local</span>
        </label>
        {!inheritGlobal && (
          <>
            <Field label={t("backups.providerLabel")}>
              <Select value={provider} onChange={(e) => setProvider((e.target as HTMLSelectElement).value as any)}>
                <option value="local">Local</option>
                <option value="google_drive">Google Drive</option>
              </Select>
            </Field>
            {provider === "google_drive" && (
              <Field label="Google Drive Folder ID">
                <Input value={folderId} placeholder="1abc... (or empty for root)" onInput={(e) => setFolderId((e.target as HTMLInputElement).value)} />
                <p class="mt-1 text-xs text-fg-muted">{t("backups.oauthHint")}</p>
              </Field>
            )}
            <Field label={t("backups.maxBackupsLabel")}>
              <Input type="number" value={maxBackups} onInput={(e) => setMaxBackups(Number((e.target as HTMLInputElement).value))} />
            </Field>
            <Field label={t("backups.maxAgeLabel")}>
              <Input value={maxAge} placeholder="Disabled" onInput={(e) => setMaxAge((e.target as HTMLInputElement).value)} />
            </Field>
          </>
        )}
        <p class="text-xs text-fg-muted">{t("backups.perServerHint")}</p>
        <Button variant="primary" onClick={save} disabled={busy}>
          {busy ? t("common.saving") : t("common.save")}
        </Button>
      </div>
    </Card>
  );
}
