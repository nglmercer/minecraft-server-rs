import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Input, formatBytes } from "./ui";
import type { FileEntry } from "../types";

/** Files an operator actually edits often enough to deserve a shortcut. */
const QUICK_EDIT = ["server.properties", "ops.json", "whitelist.json", "eula.txt"];

/** Archives the panel can unpack in place. */
const EXTRACTABLE = /\.(zip|jar|tar\.gz|tgz)$/i;

export function Files({ serverId }: { serverId: string }) {
  const [path, setPath] = useState("");
  const [entries, setEntries] = useState<FileEntry[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState<{ path: string; content: string } | null>(null);
  const [saving, setSaving] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const filePicker = useRef<HTMLInputElement | null>(null);

  const load = useCallback(
    async (target: string) => {
      setError(null);
      try {
        setEntries(await api.files(serverId, target));
        setPath(target);
      } catch (e) {
        setError(e instanceof Error ? e.message : "could not read directory");
      }
    },
    [serverId],
  );

  useEffect(() => {
    void load("");
  }, [load]);

  async function open(entry: FileEntry) {
    if (entry.directory) return load(entry.path);
    setError(null);
    try {
      setEditing(await api.readFile(serverId, entry.path));
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not read file");
    }
  }

  async function openByName(name: string) {
    setError(null);
    try {
      setEditing(await api.readFile(serverId, name));
    } catch (e) {
      setError(e instanceof Error ? e.message : `could not open ${name}`);
    }
  }

  async function save() {
    if (!editing) return;
    setSaving(true);
    try {
      await api.writeFile(serverId, editing.path, editing.content);
      setEditing(null);
      await load(path);
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not save");
    } finally {
      setSaving(false);
    }
  }

  async function remove(entry: FileEntry) {
    if (!confirm(`Delete ${entry.path}? This cannot be undone.`)) return;
    try {
      await api.deleteFile(serverId, entry.path);
      await load(path);
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not delete");
    }
  }

  async function newFolder() {
    const name = prompt("Folder name");
    if (!name) return;
    await api.mkdir(serverId, path ? `${path}/${name}` : name);
    await load(path);
  }

  async function upload(event: Event) {
    const input = event.target as HTMLInputElement;
    const files = Array.from(input.files ?? []);
    if (files.length === 0) return;

    setBusy("upload");
    setError(null);
    setNotice(null);
    try {
      await api.upload(serverId, path, files);
      setNotice(`Uploaded ${files.length} file${files.length === 1 ? "" : "s"}.`);
      await load(path);
    } catch (e) {
      setError(e instanceof Error ? e.message : "upload failed");
    } finally {
      setBusy(null);
      // Clear the picker so re-selecting the same file fires change again.
      input.value = "";
    }
  }

  async function extract(entry: FileEntry) {
    setBusy(entry.path);
    setError(null);
    setNotice(null);
    try {
      const result = await api.extract(serverId, entry.path);
      setNotice(`Extracted ${result.entries} entries from ${entry.name}.`);
      await load(path);
    } catch (e) {
      setError(e instanceof Error ? e.message : "extraction failed");
    } finally {
      setBusy(null);
    }
  }

  async function rename(entry: FileEntry) {
    const name = prompt(`Rename ${entry.name} to`, entry.name);
    if (!name || name === entry.name) return;

    const parent = entry.path.includes("/")
      ? entry.path.slice(0, entry.path.lastIndexOf("/"))
      : "";
    setBusy(entry.path);
    try {
      await api.rename(serverId, entry.path, parent ? `${parent}/${name}` : name);
      await load(path);
    } catch (e) {
      setError(e instanceof Error ? e.message : "rename failed");
    } finally {
      setBusy(null);
    }
  }

  const parent = path.includes("/") ? path.slice(0, path.lastIndexOf("/")) : "";
  const crumbs = path ? path.split("/") : [];

  if (editing) {
    return (
      <div class="flex min-h-0 flex-1 flex-col gap-3">
        <div class="flex items-center justify-between gap-3">
          <p class="truncate font-mono text-sm text-fg-muted">{editing.path}</p>
          <div class="flex gap-2">
            <Button variant="ghost" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button variant="primary" onClick={save} disabled={saving}>
              {saving ? "Saving…" : "Save"}
            </Button>
          </div>
        </div>
        {error && <Banner kind="error">{error}</Banner>}
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

  return (
    <div class="flex min-h-0 flex-1 flex-col gap-3">
      <div class="flex flex-wrap items-center justify-between gap-3">
        <nav class="flex items-center gap-1 font-mono text-sm text-fg-muted">
          <button class="hover:text-fg" onClick={() => load("")}>
            /
          </button>
          {crumbs.map((crumb, index) => (
            <span key={crumb + index} class="flex items-center gap-1">
              <button
                class="hover:text-fg"
                onClick={() => load(crumbs.slice(0, index + 1).join("/"))}
              >
                {crumb}
              </button>
              <span class="text-ink-600">/</span>
            </span>
          ))}
        </nav>
        <div class="flex gap-2">
          {QUICK_EDIT.map((name) => (
            <Button key={name} variant="ghost" class="!px-2 !py-1 !text-xs" onClick={() => openByName(name)}>
              {name}
            </Button>
          ))}
          <Button onClick={newFolder}>New folder</Button>
          <Button
            variant="primary"
            disabled={busy === "upload"}
            onClick={() => filePicker.current?.click()}
          >
            {busy === "upload" ? "Uploading…" : "Upload"}
          </Button>
          <input
            ref={filePicker}
            type="file"
            multiple
            class="hidden"
            onChange={upload}
          />
        </div>
      </div>

      {error && <Banner kind="error">{error}</Banner>}
      {notice && <Banner kind="info">{notice}</Banner>}

      <div class="min-h-0 flex-1 overflow-y-auto rounded-xl border border-ink-700 bg-ink-850">
        <table class="w-full text-sm">
          <tbody class="divide-y divide-ink-700">
            {path && (
              <tr class="hover:bg-ink-800">
                <td colspan={4} class="px-4 py-2.5">
                  <button class="font-mono text-fg-muted hover:text-fg" onClick={() => load(parent)}>
                    ../
                  </button>
                </td>
              </tr>
            )}
            {entries.map((entry) => (
              <tr key={entry.path} class="group hover:bg-ink-800">
                <td class="px-4 py-2.5">
                  <button
                    class="flex items-center gap-2 text-left font-mono hover:text-accent"
                    onClick={() => open(entry)}
                  >
                    <span class="text-fg-muted">{entry.directory ? "📁" : "📄"}</span>
                    {entry.name}
                  </button>
                </td>
                <td class="px-4 py-2.5 text-right font-mono text-xs text-fg-muted">
                  {entry.directory ? "—" : formatBytes(entry.size)}
                </td>
                <td class="px-4 py-2.5 text-right text-xs text-fg-muted">
                  {entry.modified
                    ? new Date(entry.modified * 1000).toLocaleString()
                    : "—"}
                </td>
                <td class="px-4 py-2.5">
                  <div class="flex items-center justify-end gap-2 opacity-0 transition group-hover:opacity-100">
                    {!entry.directory && EXTRACTABLE.test(entry.name) && (
                      <button
                        class="text-xs text-fg-muted hover:text-accent"
                        disabled={busy === entry.path}
                        onClick={() => extract(entry)}
                      >
                        {busy === entry.path ? "Extracting…" : "Extract"}
                      </button>
                    )}
                    {!entry.directory && (
                      <a
                        href={api.downloadUrl(serverId, entry.path)}
                        class="text-xs text-fg-muted hover:text-fg"
                      >
                        Download
                      </a>
                    )}
                    <button
                      class="text-xs text-fg-muted hover:text-fg"
                      onClick={() => rename(entry)}
                    >
                      Rename
                    </button>
                    <button
                      class="text-xs text-fg-muted hover:text-red-400"
                      onClick={() => remove(entry)}
                    >
                      Delete
                    </button>
                  </div>
                </td>
              </tr>
            ))}
            {entries.length === 0 && (
              <tr>
                <td colspan={4} class="px-4 py-8 text-center text-fg-muted">
                  This folder is empty. It fills in once the server has run once.
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      <NewFile serverId={serverId} path={path} onCreated={() => load(path)} />
    </div>
  );
}

function NewFile({
  serverId,
  path,
  onCreated,
}: {
  serverId: string;
  path: string;
  onCreated: () => void;
}) {
  const [name, setName] = useState("");

  async function create(event: Event) {
    event.preventDefault();
    if (!name.trim()) return;
    await api.writeFile(serverId, path ? `${path}/${name.trim()}` : name.trim(), "");
    setName("");
    onCreated();
  }

  return (
    <form onSubmit={create} class="flex gap-2">
      <Input
        value={name}
        placeholder="new-file.txt"
        onInput={(e) => setName((e.target as HTMLInputElement).value)}
        class="max-w-xs"
      />
      <Button type="submit">Create file</Button>
    </form>
  );
}
