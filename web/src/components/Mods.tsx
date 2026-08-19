import { useCallback, useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Input, formatBytes } from "./ui";
import type { Installed, Project, Server } from "../types";

/** Servers that cannot load anything, so the tab explains itself instead. */
const UNSUPPORTED = ["vanilla"];

export function Mods({ server }: { server: Server }) {
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Project[]>([]);
  const [installed, setInstalled] = useState<Installed[]>([]);
  const [searching, setSearching] = useState(false);
  const [installing, setInstalling] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const kind = ["fabric", "forge", "mohist", "arclight"].includes(server.core)
    ? "mods"
    : "plugins";

  const loadInstalled = useCallback(async () => {
    try {
      setInstalled(await api.installedMods(server.id));
    } catch {
      // A server with no plugins directory yet is not an error worth showing.
      setInstalled([]);
    }
  }, [server.id]);

  useEffect(() => {
    if (UNSUPPORTED.includes(server.core)) return;
    void loadInstalled();
    void search();
  }, [server.id, server.core]);

  async function search(event?: Event) {
    event?.preventDefault();
    setSearching(true);
    setError(null);
    try {
      setResults(await api.searchMods(server.id, query));
    } catch (e) {
      setError(e instanceof Error ? e.message : "search failed");
    } finally {
      setSearching(false);
    }
  }

  async function install(project: Project) {
    setInstalling(project.project_id);
    setError(null);
    setNotice(null);
    try {
      const result = await api.installMod(server.id, project.project_id);
      setNotice(
        `Installed ${result.name} ${result.version} to ${result.path}. ` +
          `Restart the server to load it.`,
      );
      await loadInstalled();
    } catch (e) {
      setError(e instanceof Error ? e.message : "install failed");
    } finally {
      setInstalling(null);
    }
  }

  async function remove(item: Installed) {
    if (!confirm(`Delete ${item.filename}?`)) return;
    try {
      await api.deleteFile(server.id, item.path);
      await loadInstalled();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not delete");
    }
  }

  if (UNSUPPORTED.includes(server.core)) {
    return (
      <div class="min-h-0 flex-1 overflow-y-auto">
        <Banner kind="info">
          A vanilla server cannot load plugins or mods. Switch this server to Paper (for
          plugins) or Fabric (for mods) under Settings — the world carries over.
        </Banner>
      </div>
    );
  }

  const installedNames = new Set(installed.map((i) => i.filename.toLowerCase()));

  return (
    <div class="min-h-0 flex-1 space-y-4 overflow-y-auto pb-6">
      {error && <Banner kind="error">{error}</Banner>}
      {notice && <Banner kind="info">{notice}</Banner>}

      <form onSubmit={search} class="flex gap-2">
        <Input
          value={query}
          placeholder={`Search Modrinth for ${server.core} ${server.version} ${kind}…`}
          onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
        />
        <Button type="submit" variant="primary" disabled={searching}>
          {searching ? "Searching…" : "Search"}
        </Button>
      </form>

      <p class="text-xs text-fg-muted">
        Results are filtered to what actually loads on {server.core} {server.version}.
      </p>

      <div class="grid gap-3">
        {results.map((project) => (
          <article
            key={project.project_id}
            class="flex items-start gap-4 rounded-xl border border-ink-700 bg-ink-850 p-4"
          >
            {project.icon_url ? (
              <img
                src={project.icon_url}
                alt=""
                loading="lazy"
                class="size-12 shrink-0 rounded-lg bg-ink-800 object-cover"
              />
            ) : (
              <div class="grid size-12 shrink-0 place-items-center rounded-lg bg-ink-800 text-fg-muted">
                ?
              </div>
            )}

            <div class="min-w-0 flex-1">
              <div class="flex items-baseline gap-2">
                <h3 class="truncate font-semibold">{project.title}</h3>
                <span class="shrink-0 text-xs text-fg-muted">
                  {project.downloads.toLocaleString()} downloads
                </span>
              </div>
              <p class="mt-0.5 line-clamp-2 text-sm text-fg-muted">{project.description}</p>
            </div>

            <Button
              variant="primary"
              class="shrink-0"
              disabled={installing === project.project_id}
              onClick={() => install(project)}
            >
              {installing === project.project_id ? "Installing…" : "Install"}
            </Button>
          </article>
        ))}

        {results.length === 0 && !searching && (
          <p class="py-6 text-center text-sm text-fg-muted">
            No results. Try a different search.
          </p>
        )}
      </div>

      <section class="space-y-2">
        <h3 class="text-sm font-semibold">
          Installed <span class="text-fg-muted">({installed.length})</span>
        </h3>
        <div class="overflow-hidden rounded-xl border border-ink-700 bg-ink-850">
          <table class="w-full text-sm">
            <tbody class="divide-y divide-ink-700">
              {installed.map((item) => (
                <tr key={item.path} class="group hover:bg-ink-800">
                  <td class="px-4 py-2.5 font-mono text-sm">
                    {item.filename}
                    {installedNames.has(item.filename.toLowerCase()) && ""}
                  </td>
                  <td class="px-4 py-2.5 text-right font-mono text-xs text-fg-muted">
                    {formatBytes(item.size)}
                  </td>
                  <td class="px-4 py-2.5 text-right">
                    <button
                      class="text-xs text-fg-muted opacity-0 transition hover:text-red-400 group-hover:opacity-100"
                      onClick={() => remove(item)}
                    >
                      Delete
                    </button>
                  </td>
                </tr>
              ))}
              {installed.length === 0 && (
                <tr>
                  <td colspan={3} class="px-4 py-6 text-center text-sm text-fg-muted">
                    Nothing installed yet.
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </section>
    </div>
  );
}
