import { useCallback, useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Input, formatBytes } from "./ui";
import type { Backup, Status } from "../types";

export function Backups({ serverId, status }: { serverId: string; status: Status }) {
  const [backups, setBackups] = useState<Backup[]>([]);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setBackups(await api.backups(serverId));
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not list backups");
    }
  }, [serverId]);

  useEffect(() => {
    void load();
  }, [load]);

  const running = status !== "offline" && status !== "crashed";

  async function create(event: Event) {
    event.preventDefault();
    setBusy("create");
    setError(null);
    setNotice(null);
    try {
      const backup = await api.createBackup(serverId, note.trim());
      setNote("");
      setNotice(`Created ${backup.id} (${formatBytes(backup.size)}).`);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "backup failed");
    } finally {
      setBusy(null);
    }
  }

  async function restore(backup: Backup) {
    if (
      !confirm(
        `Restore ${backup.id}? Files in the backup overwrite the current ones. ` +
          `Anything created since the backup is left alone.`,
      )
    )
      return;

    setBusy(backup.id);
    setError(null);
    setNotice(null);
    try {
      await api.restoreBackup(serverId, backup.id);
      setNotice(`Restored ${backup.id}.`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "restore failed");
    } finally {
      setBusy(null);
    }
  }

  async function remove(backup: Backup) {
    if (!confirm(`Delete backup ${backup.id}? This cannot be undone.`)) return;
    setBusy(backup.id);
    try {
      await api.deleteBackup(serverId, backup.id);
      await load();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not delete");
    } finally {
      setBusy(null);
    }
  }

  return (
    <div class="min-h-0 flex-1 space-y-4 overflow-y-auto pb-6">
      {error && <Banner kind="error">{error}</Banner>}
      {notice && <Banner kind="info">{notice}</Banner>}

      <form onSubmit={create} class="flex flex-wrap items-end gap-3">
        <div class="min-w-56 flex-1">
          <Input
            value={note}
            placeholder="Optional note — e.g. before the 1.21.9 upgrade"
            onInput={(e) => setNote((e.target as HTMLInputElement).value)}
          />
        </div>
        <Button type="submit" variant="primary" disabled={busy === "create"}>
          {busy === "create" ? "Archiving…" : "Take backup"}
        </Button>
      </form>

      <p class="text-xs text-fg-muted">
        Backups capture worlds, configuration and plugins. The server jar and the
        downloadable <code class="text-fg">libraries/</code>,{" "}
        <code class="text-fg">cache/</code> and <code class="text-fg">versions/</code>{" "}
        trees are skipped, since the panel can fetch those again.
        {running && " The world is flushed to disk first, so the server can stay online."}
      </p>

      <div class="overflow-hidden rounded-xl border border-ink-700 bg-ink-850">
        <table class="w-full text-sm">
          <tbody class="divide-y divide-ink-700">
            {backups.map((backup) => (
              <tr key={backup.id} class="hover:bg-ink-800">
                <td class="px-4 py-3">
                  <p class="font-mono text-sm">{backup.id}</p>
                  {backup.note && <p class="text-xs text-fg-muted">{backup.note}</p>}
                </td>
                <td class="px-4 py-3 text-right font-mono text-xs text-fg-muted">
                  {formatBytes(backup.size)}
                </td>
                <td class="px-4 py-3 text-right text-xs text-fg-muted">
                  {new Date(backup.created_at).toLocaleString()}
                </td>
                <td class="px-4 py-3">
                  <div class="flex justify-end gap-2">
                    <a
                      href={api.backupUrl(serverId, backup.id)}
                      class="rounded-lg px-2.5 py-1.5 text-xs text-fg-muted hover:bg-ink-700 hover:text-fg"
                    >
                      Download
                    </a>
                    <Button
                      class="!px-2.5 !py-1.5 !text-xs"
                      disabled={running || busy === backup.id}
                      title={running ? "Stop the server before restoring" : undefined}
                      onClick={() => restore(backup)}
                    >
                      Restore
                    </Button>
                    <Button
                      variant="ghost"
                      class="!px-2.5 !py-1.5 !text-xs"
                      disabled={busy === backup.id}
                      onClick={() => remove(backup)}
                    >
                      Delete
                    </Button>
                  </div>
                </td>
              </tr>
            ))}

            {backups.length === 0 && (
              <tr>
                <td colspan={4} class="px-4 py-8 text-center text-sm text-fg-muted">
                  No backups yet. Take one before your next upgrade.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {running && backups.length > 0 && (
        <p class="text-xs text-amber-300">
          Restoring is disabled while the server is running — unpacking a world under a
          live JVM corrupts it. Stop the server first.
        </p>
      )}
    </div>
  );
}
