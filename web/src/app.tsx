import type { ComponentChildren } from "preact";
import { lazy, Suspense } from "preact/compat";
import { Loading } from "./components/Loading";
import { useEffect, useState } from "preact/hooks";
import { api } from "./api";
import { Button } from "./components/ui";
import * as Icon from "./components/icons";
import { Tooltip } from "./components/Tooltip";
import { LanguagePicker } from "./components/LanguagePicker";
import { useI18n } from "./i18n";
import { Dashboard } from "./pages/Dashboard";
import { Login } from "./pages/Login";
import type { User } from "./types";

const BackupsSettings = lazy(() => import("./pages/BackupsSettings").then(m => ({ default: m.BackupsSettings })));

const Playit = lazy(() => import("./pages/Playit").then(m => ({ default: m.Playit })));

const Recovery = lazy(() => import("./pages/Recovery").then(m => ({ default: m.Recovery })));

const ServerDetail = lazy(() => import("./pages/ServerDetail").then(m => ({ default: m.ServerDetail })));

const Setup = lazy(() => import("./pages/Setup").then(m => ({ default: m.Setup })));

const Users = lazy(() => import("./pages/Users").then(m => ({ default: m.Users })));

/** Where the app currently is. The hash is enough for this small panel. */
type Route =
  | { page: "dashboard" }
  | { page: "server"; id: string }
  | { page: "users" }
  | { page: "playit" }
  | { page: "backups" };

function readRoute(): Route {
  const server = location.hash.match(/^#\/servers\/([^/]+)/);
  if (server) return { page: "server", id: server[1] };
  if (location.hash.startsWith("#/users")) return { page: "users" };
  if (location.hash.startsWith("#/playit")) return { page: "playit" };
  if (location.hash.startsWith("#/backups")) return { page: "backups" };
  return { page: "dashboard" };
}

function hashFor(route: Route): string {
  if (route.page === "server") return `#/servers/${route.id}`;
  if (route.page === "users") return "#/users";
  if (route.page === "playit") return "#/playit";
  if (route.page === "backups") return "#/backups";
  return "#/";
}

/** The route, derived from the hash so a reload lands back where you were. */
function useRoute(): [Route, (route: Route) => void] {
  const [route, setRoute] = useState<Route>(readRoute);

  useEffect(() => {
    const onChange = () => setRoute(readRoute());
    window.addEventListener("hashchange", onChange);
    return () => window.removeEventListener("hashchange", onChange);
  }, []);

  return [
    route,
    (next) => {
      location.hash = hashFor(next);
      setRoute(next);
    },
  ];
}

function HeaderNavButton({
  icon,
  label,
  active,
  onClick,
}: {
  icon: ComponentChildren;
  label: string;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <Tooltip label={label} align="start">
      <button
        type="button"
        title={label}
        aria-label={label}
        aria-current={active ? "page" : undefined}
        onClick={onClick}
        class={`inline-flex size-9 items-center justify-center rounded-lg text-sm transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60 lg:h-9 lg:w-auto lg:justify-start lg:gap-2 lg:px-3 ${
          active ? "bg-accent/15 text-accent" : "text-fg-muted hover:bg-ink-700 hover:text-fg"
        }`}
      >
        <span class="shrink-0">{icon}</span>
        <span class="hidden whitespace-nowrap lg:inline">{label}</span>
      </button>
    </Tooltip>
  );
}

function isSetupPath() {
  return location.pathname === "/setup" || location.hash === "#/setup";
}
function isRecoveryPath() {
  return location.pathname === "/recovery" || location.hash.startsWith("#/recovery");
}

export function App() {
  const { t } = useI18n();
  const [user, setUser] = useState<User | null>(null);
  const [ready, setReady] = useState(false);
  const [route, navigate] = useRoute();

  // Intercept path-based setup/recovery before auth check.
  const setupPath = isSetupPath();
  const recoveryPath = isRecoveryPath();

  useEffect(() => {
    if (setupPath || recoveryPath) {
      setReady(true);
      return;
    }
    // The HttpOnly cookie is intentionally invisible to JavaScript, so verify
    // the browser session directly on every page load.
    api
      .me()
      .then(setUser)
      .catch(() => {})
      .finally(() => setReady(true));
  }, []);

  useEffect(() => {
    const onLogout = () => setUser(null);
    window.addEventListener("mcpanel:logout", onLogout);
    return () => window.removeEventListener("mcpanel:logout", onLogout);
  }, []);

  if (!ready) {
    return <div class="grid h-full place-items-center text-fg-muted">{t("common.loading")}</div>;
  }

  if (setupPath) {
    return <Suspense fallback={<Loading />}><Setup onDone={() => (location.href = "/")} /></Suspense>;
  }
  if (recoveryPath) {
    return <Suspense fallback={<Loading />}><Recovery onDone={() => (location.href = "/")} /></Suspense>;
  }

  if (!user) {
    return (
      <div class="relative h-full">
        <div class="absolute right-4 top-4">
          <LanguagePicker />
        </div>
        <Login onSignedIn={setUser} />
      </div>
    );
  }

  return (
    <div class="flex h-full flex-col">
      <nav class="border-b border-ink-700 bg-ink-850 px-3 py-2.5 sm:px-6">
        <div class="mx-auto flex min-h-9 w-full max-w-7xl items-center justify-between gap-2">
          <div class="flex min-w-0 items-center gap-1.5 sm:gap-3">
            <Tooltip label={t("nav.title")} align="start">
              <button
                type="button"
                title={t("nav.title")}
                aria-label={t("nav.title")}
                class="inline-flex h-9 shrink-0 items-center gap-2 rounded-lg text-sm font-semibold focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
                onClick={() => navigate({ page: "dashboard" })}
              >
                <span class="grid size-7 place-items-center rounded-md bg-accent text-sm font-bold text-ink-950">
                  M
                </span>
                <span class="hidden whitespace-nowrap lg:inline">{t("nav.title")}</span>
              </button>
            </Tooltip>

            {user.admin && (
              <div class="flex items-center gap-0.5 rounded-xl border border-ink-700 bg-ink-900/50 p-1">
                <HeaderNavButton
                  icon={<Icon.Users size={16} />}
                  label={t("nav.accounts")}
                  active={route.page === "users"}
                  onClick={() => navigate({ page: "users" })}
                />
                <HeaderNavButton
                  icon={<Icon.Globe size={16} />}
                  label={t("nav.playit")}
                  active={route.page === "playit"}
                  onClick={() => navigate({ page: "playit" })}
                />
                <HeaderNavButton
                  icon={<Icon.Archive size={16} />}
                  label={t("nav.backups")}
                  active={route.page === "backups"}
                  onClick={() => navigate({ page: "backups" })}
                />
              </div>
            )}
          </div>

          <div class="flex shrink-0 items-center gap-1.5 text-sm">
            <Tooltip label={user.admin ? `${user.username} · ${t("nav.admin")}` : user.username}>
              <span
                tabIndex={0}
                aria-label={user.admin ? `${user.username} · ${t("nav.admin")}` : user.username}
                class="inline-flex h-9 items-center justify-center gap-1.5 rounded-lg px-2 text-fg-muted outline-none transition-colors hover:bg-ink-700 hover:text-fg focus-visible:ring-2 focus-visible:ring-accent/60"
              >
                <Icon.User size={16} />
                <span class="hidden whitespace-nowrap lg:inline">
                  {user.username}
                  {user.admin && <span class="ml-1.5 text-xs text-accent">{t("nav.admin")}</span>}
                </span>
              </span>
            </Tooltip>

            <LanguagePicker />

            <Tooltip label={t("nav.signOut")}>
              <Button
                type="button"
                variant="ghost"
                square
                aria-label={t("nav.signOut")}
                title={t("nav.signOut")}
                icon={<Icon.LogOut size={18} />}
                class="size-9 lg:h-auto lg:w-auto lg:px-4 lg:py-2"
                onClick={async () => {
                  await api.logout();
                  setUser(null);
                  navigate({ page: "dashboard" });
                }}
              >
                <span class="hidden lg:inline">{t("nav.signOut")}</span>
              </Button>
            </Tooltip>
          </div>
        </div>
      </nav>

      <main class="min-h-0 min-w-0 flex-1 overflow-x-hidden overflow-y-auto">
        <Suspense fallback={<Loading />}>
        {route.page === "server" && (
          <ServerDetail
            id={route.id}
            user={user}
            onBack={() => navigate({ page: "dashboard" })}
          />
        )}
        {route.page === "users" && <Users currentUser={user.username} />}
        {route.page === "playit" && <Playit />}
        {route.page === "backups" && <BackupsSettings />}
        {route.page === "dashboard" && (
          <Dashboard user={user} onOpen={(id) => navigate({ page: "server", id })} />
        )}
        </Suspense>
      </main>
    </div>
  );
}
