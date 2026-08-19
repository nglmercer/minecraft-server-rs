import { useCallback, useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { useT } from "../i18n";
import { useDialogs } from "./Modal";
import { useToast } from "./Toast";
import { Actions, Button, Empty, Input, formatBytes } from "./ui";
import type { Backup, Status } from "../types";

export function Backups({ serverId, status }: { serverId: string; status: Status }) {
  const t = useT();
  const dialogs = useDialogs();
  const toast = useToast();

  const [backups, setBackups] = useState<Backup[]>([]);
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const fail = useCallback(
    (error: unknown) =>
      toast.error(error instanceof Error ? error.message : t("errors.actionFailed")),
    [toast, t],
  );

  const load = useCallback(async () => {
    try {
      setBackups(await api.backups(serverId));
    } catch (e) {
      fail(e);
    }
  }, [serverId, fail]);

  useEffect(() => {
    void load();
  }, [load]);

  const running = status !== "offline" && status !== "crashed";

  async function create(event: Event) {
    event.preventDefault();
    setBusy("create");
    try {
      const backup = await api.createBackup(serverId, note.trim());
      setNote("");
      toast.success(t("backups.created", { id: backup.id, size: formatBytes(backup.size) }));
      await load();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  }

  async function restore(backup: Backup) {
    const confirmed = await dialogs.confirm({
      title: t("backups.restoreTitle", { id: backup.id }),
      body: t("backups.restoreBody"),
      confirmLabel: t("common.restore"),
    });
    if (!confirmed) return;

    setBusy(backup.id);
    try {
      await api.restoreBackup(serverId, backup.id);
      toast.success(t("backups.restored", { id: backup.id }));
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  }

  async function remove(backup: Backup) {
    const confirmed = await dialogs.confirm({
      title: t("backups.deleteTitle", { id: backup.id }),
      body: t("backups.deleteBody"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    setBusy(backup.id);
    try {
      await api.deleteBackup(serverId, backup.id);
      await load();
    } catch (e) {
      fail(e);
    } finally {
      setBusy(null);
    }
  }

  return (
    <div class="min-h-0 flex-1 space-y-4 overflow-y-auto pb-6">
      <form onSubmit={create} class="flex flex-wrap items-end gap-3">
        <div class="min-w-56 flex-1">
          <Input
            value={note}
            placeholder={t("backups.notePlaceholder")}
            onInput={(e) => setNote((e.target as HTMLInputElement).value)}
          />
        </div>
        <Button type="submit" variant="primary" disabled={busy === "create"}>
          {busy === "create" ? t("backups.taking") : t("backups.take")}
        </Button>
      </form>

      <p class="text-xs leading-relaxed text-fg-muted">
        {t("backups.explain")}
        {running && ` ${t("backups.explainOnline")}`}
      </p>

      <div class="overflow-hidden rounded-xl border border-ink-700 bg-ink-850">
        <table class="w-full text-sm">
          <tbody class="divide-y divide-ink-700">
            {backups.map((backup) => (
              <tr key={backup.id} class="hover:bg-ink-800">
                <td class="px-4 py-3">
                  <p class="font-mono text-sm">{backup.id}</p>
                  {backup.note && <p class="mt-0.5 text-xs text-fg-muted">{backup.note}</p>}
                </td>
                <td class="px-4 py-3 text-right font-mono text-xs text-fg-muted">
                  {formatBytes(backup.size)}
                </td>
                <td class="whitespace-nowrap px-4 py-3 text-right text-xs text-fg-muted">
                  {new Date(backup.created_at).toLocaleString()}
                </td>
                <td class="px-4 py-3">
                  <Actions>
                    <a
                      href={api.backupUrl(serverId, backup.id)}
                      class="rounded-lg px-2.5 py-1.5 text-xs text-fg-muted hover:bg-ink-700 hover:text-fg"
                    >
                      {t("common.download")}
                    </a>
                    <Button
                      class="!px-2.5 !py-1.5 !text-xs"
                      disabled={running || busy === backup.id}
                      title={running ? t("backups.mustStop") : undefined}
                      onClick={() => restore(backup)}
                    >
                      {t("common.restore")}
                    </Button>
                    <Button
                      variant="ghost"
                      class="!px-2.5 !py-1.5 !text-xs"
                      disabled={busy === backup.id}
                      onClick={() => remove(backup)}
                    >
                      {t("common.delete")}
                    </Button>
                  </Actions>
                </td>
              </tr>
            ))}

            {backups.length === 0 && (
              <tr>
                <td colspan={4}>
                  <Empty>{t("backups.empty")}</Empty>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {running && backups.length > 0 && (
        <p class="text-xs leading-relaxed text-amber-300">{t("backups.runningWarning")}</p>
      )}
    </div>
  );
}
