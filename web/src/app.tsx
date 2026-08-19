import { useEffect, useState } from "preact/hooks";
import { api, token } from "./api";
import { Button } from "./components/ui";
import { Dashboard } from "./pages/Dashboard";
import { Login } from "./pages/Login";
import { ServerDetail } from "./pages/ServerDetail";
import { Users } from "./pages/Users";
import type { User } from "./types";

/** Where the app currently is. Only two routes carry state, so no router. */
type Route = { page: "dashboard" } | { page: "server"; id: string } | { page: "users" };

function readRoute(): Route {
  const server = location.hash.match(/^#\/servers\/([^/]+)/);
  if (server) return { page: "server", id: server[1] };
  if (location.hash.startsWith("#/users")) return { page: "users" };
  return { page: "dashboard" };
}

function hashFor(route: Route): string {
  if (route.page === "server") return `#/servers/${route.id}`;
  if (route.page === "users") return "#/users";
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

export function App() {
  const [user, setUser] = useState<User | null>(null);
  const [ready, setReady] = useState(false);
  const [route, navigate] = useRoute();

  useEffect(() => {
    // A stored token may have expired while the tab was closed; verify it once.
    if (!token()) {
      setReady(true);
      return;
    }
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
    return <div class="grid h-full place-items-center text-fg-muted">Loading…</div>;
  }

  if (!user) {
    return <Login onSignedIn={setUser} />;
  }

  return (
    <div class="flex h-full flex-col">
      <nav class="flex items-center justify-between border-b border-ink-700 bg-ink-850 px-6 py-3">
        <div class="flex items-center gap-5">
          <button
            class="flex items-center gap-2.5"
            onClick={() => navigate({ page: "dashboard" })}
          >
            <span class="grid size-7 place-items-center rounded-md bg-accent text-sm font-bold text-ink-950">
              M
            </span>
            <span class="text-sm font-semibold">Minecraft Panel</span>
          </button>

          {user.admin && (
            <button
              class={`text-sm transition-colors ${
                route.page === "users" ? "text-fg" : "text-fg-muted hover:text-fg"
              }`}
              onClick={() => navigate({ page: "users" })}
            >
              Accounts
            </button>
          )}
        </div>

        <div class="flex items-center gap-3 text-sm">
          <span class="text-fg-muted">
            {user.username}
            {user.admin && <span class="ml-1.5 text-xs text-accent">admin</span>}
          </span>
          <Button
            variant="ghost"
            onClick={async () => {
              await api.logout();
              setUser(null);
              navigate({ page: "dashboard" });
            }}
          >
            Sign out
          </Button>
        </div>
      </nav>

      <main class="min-h-0 flex-1 overflow-y-auto">
        {route.page === "server" && (
          <ServerDetail
            id={route.id}
            user={user}
            onBack={() => navigate({ page: "dashboard" })}
          />
        )}
        {route.page === "users" && <Users currentUser={user.username} />}
        {route.page === "dashboard" && (
          <Dashboard user={user} onOpen={(id) => navigate({ page: "server", id })} />
        )}
      </main>
    </div>
  );
}
