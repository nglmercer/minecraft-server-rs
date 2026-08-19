import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useT } from "../i18n";
import { MenuButton, useMenu, type MenuItem } from "./Menu";
import { useDialogs } from "./Modal";
import { useToast } from "./Toast";
import { Actions, Button, Empty, formatBytes } from "./ui";
import * as Icon from "./icons";
import type { FileEntry } from "../types";

/** Files an operator edits often enough to deserve a shortcut. */
const QUICK_EDIT = ["server.properties", "ops.json", "whitelist.json", "eula.txt"];

/** Archives the panel can unpack in place. */
const EXTRACTABLE = /\.(zip|jar|tar\.gz|tgz)$/i;

export function Files({ serverId }: { serverId: string }) {
  const t = useT();
  const dialogs = useDialogs();
  const toast = useToast();
  const menu = useMenu();

  const [path, setPath] = useState("");
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [editing, setEditing] = useState<{ path: string; content: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [sizes, setSizes] = useState<Record<string, number>>({});
  const filePicker = useRef<HTMLInputElement | null>(null);
  // Read by the size request to decide whether its reply is still wanted. A
  // state updater must stay pure, so it cannot double as a way to read state.
  const currentPath = useRef("");

  const fail = useCallback(
    (error: unknown, fallback: string) =>
      toast.error(error instanceof Error ? error.message : fallback),
    [toast],
  );

  const load = useCallback(
    async (target: string) => {
      try {
        setEntries(await api.files(serverId, target));
        setPath(target);
        currentPath.current = target;
        setSizes({});

        // Measured separately so the listing is not held up by a walk of
        // world/ or libraries/. A stale reply is dropped rather than applied
        // to whatever folder the operator has since moved to.
        api
          .directorySizes(serverId, target)
          .then((measured) => {
            if (currentPath.current !== target) return;
            setSizes(Object.fromEntries(measured.map((d) => [d.path, d.bytes])));
          })
          .catch(() => {});
      } catch (e) {
        fail(e, t("errors.generic"));
      }
    },
    [serverId, fail, t],
  );

  useEffect(() => {
    void load("");
  }, [load]);

  /** A directory's measured size, or a placeholder while it is being walked. */
  function sizeOf(entry: FileEntry) {
    if (!entry.directory) return formatBytes(entry.size);
    const bytes = sizes[entry.path];
    return bytes === undefined ? "…" : formatBytes(bytes);
  }

  /** Join a name onto the directory currently being browsed. */
  const under = (name: string) => (path ? `${path}/${name}` : name);

  async function open(entry: FileEntry) {
    if (entry.directory) return load(entry.path);
    try {
      setEditing(await api.readFile(serverId, entry.path));
    } catch (e) {
      fail(e, t("files.tooLarge"));
    }
  }

  async function openByName(name: string) {
    try {
      setEditing(await api.readFile(serverId, name));
    } catch (e) {
      fail(e, t("errors.generic"));
    }
  }

  async function save() {
    if (!editing) return;
    setSaving(true);
    try {
      await api.writeFile(serverId, editing.path, editing.content);
      toast.success(t("common.save"));
      setEditing(null);
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    } finally {
      setSaving(false);
    }
  }

  async function remove(entry: FileEntry) {
    const confirmed = await dialogs.confirm({
      title: t("files.deleteTitle", { name: entry.name }),
      body: t("files.deleteBody"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    try {
      await api.deleteFile(serverId, entry.path);
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    }
  }

  async function newFolder() {
    const name = await dialogs.prompt({
      title: t("files.newFolderTitle"),
      label: t("files.newFolderLabel"),
      confirmLabel: t("common.create"),
    });
    if (!name) return;

    try {
      await api.mkdir(serverId, under(name));
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    }
  }

  async function newFile() {
    const name = await dialogs.prompt({
      title: t("files.newFile"),
      label: t("common.name"),
      placeholder: t("files.newFilePlaceholder"),
      confirmLabel: t("common.create"),
    });
    if (!name) return;

    try {
      await api.writeFile(serverId, under(name), "");
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    }
  }

  async function upload(event: Event) {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    if (files.length === 0) return;

    setBusy("upload");
    try {
      await api.upload(serverId, path, files);
      toast.success(t("files.uploaded", { count: files.length }));
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    } finally {
      setBusy(null);
      // Cleared so re-picking the same file fires change again.
      input.value = "";
    }
  }

  async function extract(entry: FileEntry) {
    setBusy(entry.path);
    try {
      const result = await api.extract(serverId, entry.path);
      toast.success(t("files.extracted", { count: result.entries, name: entry.name }));
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    } finally {
      setBusy(null);
    }
  }

  async function rename(entry: FileEntry) {
    const name = await dialogs.prompt({
      title: t("files.renameTitle", { name: entry.name }),
      label: t("files.renameLabel"),
      initial: entry.name,
      confirmLabel: t("common.rename"),
    });
    if (!name || name === entry.name) return;

    const parent = entry.path.includes("/")
      ? entry.path.slice(0, entry.path.lastIndexOf("/"))
      : "";

    try {
      await api.rename(serverId, entry.path, parent ? `${parent}/${name}` : name);
      await load(path);
    } catch (e) {
      fail(e, t("errors.generic"));
    }
  }

  /** The actions for one entry, shared by the trigger button and long-press. */
  function actionsFor(entry: FileEntry): MenuItem[] {
    const items: MenuItem[] = [
      { label: entry.directory ? t("files.openFolder") : t("files.edit"), onSelect: () => open(entry) },
    ];

    if (!entry.directory) {
      if (EXTRACTABLE.test(entry.name)) {
        items.push({ label: t("common.extract"), onSelect: () => extract(entry) });
      }
      items.push({
        label: t("common.download"),
        onSelect: () => {
          api.download(serverId, entry.path).catch((e) => fail(e, t("errors.generic")));
        },
      });
    }

    items.push({ label: t("common.rename"), onSelect: () => rename(entry) });
    items.push({ label: t("common.delete"), danger: true, onSelect: () => remove(entry) });

    return items;
  }

  const openMenu = (event: MouseEvent, entry: FileEntry) =>
    menu.open(event, actionsFor(entry), entry.name);

  if (editing) {
    return (
      <div class="flex min-h-0 flex-1 flex-col gap-3">
        <div class="flex items-center justify-between gap-3">
          <p class="truncate font-mono text-sm text-fg-muted">{editing.path}</p>
          <Actions>
            <Button variant="ghost" onClick={() => setEditing(null)}>
              {t("common.cancel")}
            </Button>
            <Button variant="primary" onClick={save} disabled={saving}>
              {saving ? t("common.saving") : t("common.save")}
            </Button>
          </Actions>
        </div>
        <textarea
          value={editing.content}
          onInput={(e) =>
            setEditing({ ...editing, content: (e.target as HTMLTextAreaElement).value })
          }
          spellcheck={false}
          class="min-h-0 flex-1 resize-none rounded-xl border border-ink-700 bg-ink-950 p-4 font-mono text-[13px] leading-relaxed text-fg focus:border-accent focus:outline-none"
        />
      </div>
    );
  }

  const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const crumbs = path ? path.split("/") : [];

  return (
    <div class="flex min-h-0 flex-1 flex-col gap-3">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <nav class="flex items-center gap-1 font-mono text-sm text-fg-muted">
          <button class="rounded px-1 hover:text-fg" onClick={() => load("")}>
            /
          </button>
          {crumbs.map((crumb, index) => (
            <span key={`${crumb}-${index}`} class="flex items-center gap-1">
              <button
                class="rounded px-1 hover:text-fg"
                onClick={() => load(crumbs.slice(0, index + 1).join("/"))}
              >
                {crumb}
              </button>
              <span class="text-ink-600">/</span>
            </span>
          ))}
        </nav>

        <Actions>
          <Button icon={<Icon.File size={15} />} onClick={newFile}>
            {t("files.newFile")}
          </Button>
          <Button icon={<Icon.FolderOpen size={15} />} onClick={newFolder}>
            {t("files.newFolder")}
          </Button>
          <Button
            variant="primary"
            icon={<Icon.Upload size={15} />}
            disabled={busy === "upload"}
            onClick={() => filePicker.current?.click()}
          >
            {busy === "upload" ? t("common.uploading") : t("common.upload")}
          </Button>
          <input ref={filePicker} type="file" multiple class="hidden" onChange={upload} />
        </Actions>
      </div>

      {path === "" && (
        <div class="flex flex-wrap gap-1.5">
          {QUICK_EDIT.map((name) => (
            <button
              key={name}
              onClick={() => openByName(name)}
              class="rounded-lg border border-ink-700 px-2.5 py-1 font-mono text-xs text-fg-muted transition-colors hover:border-accent/50 hover:text-fg"
            >
              {name}
            </button>
          ))}
        </div>
      )}

      <div class="min-h-0 flex-1 overflow-y-auto rounded-xl border border-ink-700 bg-ink-850">
        <table class="w-full text-sm">
          <thead class="sticky top-0 bg-ink-850 text-left text-xs uppercase tracking-wider text-fg-muted">
            <tr class="border-b border-ink-700">
              <th class="px-4 py-2.5 font-medium">{t("common.name")}</th>
              <th class="hidden px-4 py-2.5 text-right font-medium sm:table-cell">
                {t("files.size")}
              </th>
              <th class="hidden px-4 py-2.5 text-right font-medium md:table-cell">
                {t("files.modified")}
              </th>
              <th class="px-4 py-2.5" />
            </tr>
          </thead>
          <tbody class="divide-y divide-ink-700">
            {path && (
              <tr class="hover:bg-ink-800">
                <td colspan={4} class="px-4 py-2.5">
                  <button class="py-1 font-mono text-fg-muted hover:text-fg" onClick={() => load(parent)}>
                    ../
                  </button>
                </td>
              </tr>
            )}

            {entries.map((entry) => (
              <tr
                key={entry.path}
                // Right-click on desktop, long-press on touch: both arrive here.
                onContextMenu={(event) => openMenu(event as unknown as MouseEvent, entry)}
                class="group select-none hover:bg-ink-800 [-webkit-touch-callout:none]"
              >
                <td class="px-4 py-2.5">
                  <button
                    class="flex w-full items-center gap-2 text-left font-mono hover:text-accent"
                    onClick={() => open(entry)}
                  >
                    <span class="shrink-0 text-fg-muted">
                      {entry.directory ? <Icon.Folder size={16} /> : <Icon.File size={16} />}
                    </span>
                    <span class="truncate">{entry.name}</span>
                  </button>
                  {/* The size lives here on narrow screens, where its column is dropped. */}
                  <span class="ml-6 font-mono text-xs text-fg-muted sm:hidden">
                    {sizeOf(entry)}
                    {busy === entry.path && ` · ${t("common.extracting")}`}
                  </span>
                </td>
                <td class="hidden px-4 py-2.5 text-right font-mono text-xs text-fg-muted sm:table-cell">
                  {sizeOf(entry)}
                </td>
                <td class="hidden whitespace-nowrap px-4 py-2.5 text-right text-xs text-fg-muted md:table-cell">
                  {entry.modified ? new Date(entry.modified * 1000).toLocaleString() : "—"}
                </td>
                <td class="py-2.5 pr-2">
                  <div class="flex justify-end">
                    <MenuButton
                      label={t("files.actionsFor", { name: entry.name })}
                      onOpen={(event) => openMenu(event, entry)}
                    />
                  </div>
                </td>
              </tr>
            ))}

            {entries.length === 0 && (
              <tr>
                <td colspan={4}>
                  <Empty>{t("files.empty")}</Empty>
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
