import { useCallback, useEffect, useRef, useState } from "preact/hooks";
import { api } from "../api";
import { useT } from "../i18n";
import { MenuButton, useMenu } from "./Menu";
import { useDialogs } from "./Modal";
import { useToast } from "./Toast";
import { Banner, Button, Empty, Input, formatBytes } from "./ui";
import * as Icon from "./icons";
import type { Installed, Project, Server } from "../types";

/** Flavours that cannot load anything, so the tab explains itself instead. */
const UNSUPPORTED = ["vanilla"];

/** Flavours whose add-ons are mods rather than plugins. */
const MODDED = ["fabric", "forge", "mohist", "arclight"];

export function Mods({ server }: { server: Server }) {
  const t = useT();
  const dialogs = useDialogs();
  const toast = useToast();
  const menu = useMenu();

  const [view, setView] = useState<"installed" | "browse">("installed");
  const [loadError, setLoadError] = useState(false);
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Project[]>([]);
  const [installed, setInstalled] = useState<Installed[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState(false);
  const searchSequence = useRef(0);
  const [installing, setInstalling] = useState<string | null>(null);

  const supported = !UNSUPPORTED.includes(server.core);
  const kind = MODDED.includes(server.core)
    ? t("plugins.kindMods")
    : t("plugins.kindPlugins");

  const fail = useCallback(
    (error: unknown) =>
      toast.error(
        error instanceof Error ? error.message : t("errors.actionFailed"),
      ),
    [toast, t],
  );

  const loadInstalled = useCallback(async () => {
    try {
      setInstalled(await api.installedMods(server.id));
      setLoadError(false);
    } catch {
      setLoadError(true);
    }
  }, [server.id]);

  const search = useCallback(
    async (term: string) => {
      const sequence = ++searchSequence.current;
      setSearching(true);
      setSearchError(false);
      try {
        const next = await api.searchMods(server.id, term);
        if (sequence === searchSequence.current) setResults(next);
      } catch (e) {
        if (sequence === searchSequence.current) {
          setSearchError(true);
          fail(e);
        }
      } finally {
        if (sequence === searchSequence.current) setSearching(false);
      }
    },
    [server.id, fail],
  );

  useEffect(() => {
    if (!supported) return;
    void loadInstalled();
  }, [supported, loadInstalled]);

  useEffect(() => {
    if (supported && view === "browse") void search(query);
    return () => {
      searchSequence.current += 1;
    };
  }, [view, server.id]);

  async function install(project: Project) {
    setInstalling(project.project_id);
    try {
      const result = await api.installMod(server.id, project.project_id);
      toast.success(
        t("plugins.installedOk", {
          name: result.name,
          version: result.version,
          path: result.path,
        }),
      );
      await loadInstalled();
      setView("installed");
    } catch (e) {
      fail(e);
    } finally {
      setInstalling(null);
    }
  }

  async function remove(item: Installed) {
    const confirmed = await dialogs.confirm({
      title: t("plugins.deleteTitle", { name: item.filename }),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    try {
      await api.deleteFile(server.id, item.path);
      await loadInstalled();
      setView("installed");
    } catch (e) {
      fail(e);
    }
  }

  if (!supported) {
    return (
      <div class="min-h-0 flex-1 overflow-y-auto">
        <Banner kind="info">{t("plugins.unsupported")}</Banner>
      </div>
    );
  }

  return (
    <div class="min-h-0 flex-1 space-y-4 overflow-y-auto pb-6">
      <nav aria-label={t("plugins.sections")} class="flex flex-wrap gap-2">
        <Button
          aria-pressed={view === "installed"}
          onClick={() => setView("installed")}
        >
          {t("plugins.installed")} ({installed.length})
        </Button>
        <Button
          aria-pressed={view === "browse"}
          onClick={() => setView("browse")}
        >
          {t("plugins.browse")}
        </Button>
      </nav>
      {server.status === "online" && (
        <Banner kind="info">{t("plugins.restartHint")}</Banner>
      )}
      {view === "browse" && (
        <>
          <form
            onSubmit={(event) => {
              event.preventDefault();
              void search(query);
            }}
            class="flex gap-2"
          >
            <Input
              value={query}
              placeholder={t("plugins.searchPlaceholder", {
                core: server.core,
                version: server.version,
                kind,
              })}
              onInput={(e) => setQuery((e.target as HTMLInputElement).value)}
            />
            <Button
              type="submit"
              variant="primary"
              icon={<Icon.Search size={15} />}
              disabled={searching}
            >
              {searching ? t("common.searching") : t("common.search")}
            </Button>
          </form>

          <p class="text-xs text-fg-muted">
            {t("plugins.scoped", {
              core: server.core,
              version: server.version,
            })}
          </p>

          {searchError && (
            <Banner kind="error">{t("plugins.searchError")}</Banner>
          )}
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
                  <div class="flex flex-wrap items-baseline gap-2">
                    <h3 class="truncate font-semibold">{project.title}</h3>
                    <span class="shrink-0 text-xs text-fg-muted">
                      {t("plugins.downloads", {
                        count: project.downloads.toLocaleString(),
                      })}
                    </span>
                  </div>
                  <p class="mt-0.5 line-clamp-2 text-sm text-fg-muted">
                    {project.description}
                  </p>
                </div>

                <Button
                  variant="primary"
                  icon={<Icon.Download size={15} />}
                  class="shrink-0"
                  disabled={installing === project.project_id}
                  onClick={() => install(project)}
                >
                  {installing === project.project_id
                    ? t("common.installing")
                    : t("common.install")}
                </Button>
              </article>
            ))}

            {results.length === 0 && !searching && !searchError && (
              <Empty>{t("plugins.noResults")}</Empty>
            )}
          </div>
        </>
      )}
      {view === "installed" && (
        <section class="space-y-2">
          {loadError && (
            <Banner kind="error">
              {t("plugins.loadError")}{" "}
              <Button onClick={loadInstalled}>
                {t("compatibility.retry")}
              </Button>
            </Banner>
          )}
          <h3 class="text-sm font-semibold">
            {t("plugins.installed")}{" "}
            <span class="text-fg-muted">({installed.length})</span>
          </h3>

          <div class="overflow-hidden rounded-xl border border-ink-700 bg-ink-850">
            <table class="w-full text-sm">
              <tbody class="divide-y divide-ink-700">
                {installed.map((item) => (
                  <tr
                    key={item.path}
                    onContextMenu={(event) =>
                      menu.open(
                        event as unknown as MouseEvent,
                        [
                          {
                            label: t("common.delete"),
                            danger: true,
                            onSelect: () => remove(item),
                          },
                        ],
                        item.filename,
                      )
                    }
                    class="select-none hover:bg-ink-800 [-webkit-touch-callout:none]"
                  >
                    <td class="px-4 py-2.5">
                      <p class="break-all font-mono text-sm">{item.filename}</p>
                      <p class="font-mono text-xs text-fg-muted sm:hidden">
                        {formatBytes(item.size)}
                      </p>
                    </td>
                    <td class="hidden px-4 py-2.5 text-right font-mono text-xs text-fg-muted sm:table-cell">
                      {formatBytes(item.size)}
                    </td>
                    <td class="py-2.5 pr-2">
                      <div class="flex justify-end">
                        <MenuButton
                          label={t("files.actionsFor", { name: item.filename })}
                          onOpen={(event) =>
                            menu.open(
                              event,
                              [
                                {
                                  label: t("common.delete"),
                                  danger: true,
                                  onSelect: () => remove(item),
                                },
                              ],
                              item.filename,
                            )
                          }
                        />
                      </div>
                    </td>
                  </tr>
                ))}

                {installed.length === 0 && !loadError && (
                  <tr>
                    <td colspan={3}>
                      <Empty>{t("plugins.nothingInstalled")}</Empty>
                    </td>
                  </tr>
                )}
              </tbody>
            </table>
          </div>
        </section>
      )}
    </div>
  );
}
