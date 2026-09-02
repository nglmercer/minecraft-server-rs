import { useCallback, useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { useT } from "../i18n";
import { MenuButton, useMenu, type MenuItem } from "./Menu";
import { useDialogs } from "./Modal";
import { useToast } from "./Toast";
import { Button, Empty, InfoTooltip, Input, formatBytes } from "./ui";
import * as Icon from "./icons";
import type { Backup, Status } from "../types";

export function Backups({ serverId, status }: { serverId: string; status: Status }) {
  const t = useT();
  const dialogs = useDialogs();
  const toast = useToast();
  const menu = useMenu();

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

  function backupSize(b: Backup): number {
    // Backend returns StoredBackup with size_bytes; older data may have size
    return (b as unknown as { size_bytes?: number }).size_bytes ?? b.size ?? 0;
  }

  async function create(event: Event) {
    event.preventDefault();
    setBusy("create");
    try {
      const backup = await api.createBackup(serverId, note.trim());
      setNote("");
      toast.success(t("backups.created", { id: backup.id, size: formatBytes(backupSize(backup)) }));
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

  function actionsFor(backup: Backup): MenuItem[] {
    return [
      {
        label: t("common.download"),
        onSelect: () => {
          api.downloadBackup(serverId, backup.id).catch(fail);
        },
      },
      {
        label: t("common.restore"),
        onSelect: () => restore(backup),
        // Unpacking a world under a live JVM corrupts it.
        disabled: running || busy === backup.id,
      },
      { label: t("common.delete"), danger: true, onSelect: () => remove(backup) },
    ];
  }

  return (
    <div class="min-h-0 flex-1 space-y-4 overflow-y-auto pb-6">
      <form onSubmit={create} class="flex flex-wrap items-center gap-3">
        <div class="min-w-56 flex-1">
          <Input
            value={note}
            placeholder={t("backups.notePlaceholder")}
            onInput={(e) => setNote((e.target as HTMLInputElement).value)}
          />
        </div>
        <Button type="submit" variant="primary" icon={<Icon.Archive size={15} />} disabled={busy === "create"}>
          {busy === "create" ? t("backups.taking") : t("backups.take")}
        </Button>
        <InfoTooltip text={`${t("backups.explain")}${running ? ` ${t("backups.explainOnline")}` : ""}`} />
      </form>

      <div class="overflow-hidden rounded-xl border border-ink-700 bg-ink-850">
        <table class="w-full text-sm">
          <tbody class="divide-y divide-ink-700">
            {backups.map((backup) => (
              <tr
                key={backup.id}
                onContextMenu={(event) =>
                  menu.open(event as unknown as MouseEvent, actionsFor(backup), backup.id)
                }
                class="select-none hover:bg-ink-800 [-webkit-touch-callout:none]"
              >
                <td class="px-4 py-3">
                  <p class="font-mono text-sm">{backup.id}</p>
                  {backup.note && <p class="mt-0.5 text-xs text-fg-muted">{backup.note}</p>}
                  <p class="mt-0.5 font-mono text-xs text-fg-muted sm:hidden">
                    {formatBytes(backupSize(backup))} · {new Date(backup.created_at).toLocaleString()}
                  </p>
                </td>
                <td class="hidden px-4 py-3 text-right font-mono text-xs text-fg-muted sm:table-cell">
                  {formatBytes(backupSize(backup))}
                </td>
                <td class="hidden whitespace-nowrap px-4 py-3 text-right text-xs text-fg-muted md:table-cell">
                  {new Date(backup.created_at).toLocaleString()}
                </td>
                <td class="py-3 pr-2">
                  <div class="flex justify-end">
                    <MenuButton
                      label={t("files.actionsFor", { name: backup.id })}
                      onOpen={(event) =>
                        menu.open(event, actionsFor(backup), backup.id)
                      }
                    />
                  </div>
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
