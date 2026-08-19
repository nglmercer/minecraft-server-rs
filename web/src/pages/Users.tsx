import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Banner, Button, Card, Field, Input } from "../components/ui";
import type { PanelUser, Server } from "../types";

export function Users({ currentUser }: { currentUser: string }) {
  const [users, setUsers] = useState<PanelUser[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);

  async function refresh() {
    try {
      const [list, allServers] = await Promise.all([api.users(), api.servers()]);
      setUsers(list);
      setServers(allServers);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not load accounts");
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  async function remove(user: PanelUser) {
    if (!confirm(`Delete the account "${user.username}"?`)) return;
    try {
      await api.deleteUser(user.username);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not delete");
    }
  }

  async function toggleAdmin(user: PanelUser) {
    try {
      await api.updateUser(user.username, { admin: !user.admin });
      setNotice(`${user.username} is now ${user.admin ? "a regular user" : "an admin"}.`);
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not update");
    }
  }

  async function resetPassword(user: PanelUser) {
    const password = prompt(`New password for ${user.username} (at least 8 characters)`);
    if (!password) return;
    try {
      await api.updateUser(user.username, { password });
      setNotice(`Password changed for ${user.username}. Their sessions were signed out.`);
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not update");
    }
  }

  async function setAccess(user: PanelUser, serverId: string, allowed: boolean) {
    const next = allowed
      ? [...user.servers, serverId]
      : user.servers.filter((s) => s !== serverId);
    try {
      await api.updateUser(user.username, { servers: next });
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not update access");
    }
  }

  return (
    <div class="mx-auto w-full max-w-4xl space-y-6 px-6 py-8">
      <header class="flex items-end justify-between gap-4">
        <div>
          <h1 class="text-2xl font-semibold">Accounts</h1>
          <p class="text-sm text-fg-muted">{users.length} accounts</p>
        </div>
        <Button variant="primary" onClick={() => setCreating((v) => !v)}>
          {creating ? "Cancel" : "New account"}
        </Button>
      </header>

      {error && <Banner kind="error">{error}</Banner>}
      {notice && <Banner kind="info">{notice}</Banner>}

      {creating && (
        <CreateUser
          servers={servers}
          onCreated={async () => {
            setCreating(false);
            await refresh();
          }}
        />
      )}

      <div class="grid gap-3">
        {users.map((user) => (
          <Card key={user.username}>
            <div class="flex flex-wrap items-start justify-between gap-4">
              <div>
                <p class="font-semibold">
                  {user.username}
                  {user.username === currentUser && (
                    <span class="ml-2 text-xs text-fg-muted">(you)</span>
                  )}
                  {user.admin && <span class="ml-2 text-xs text-accent">admin</span>}
                </p>
                <p class="mt-0.5 text-xs text-fg-muted">
                  {user.admin
                    ? "Full access to every server and account."
                    : `${user.servers.length} server${user.servers.length === 1 ? "" : "s"} granted`}
                </p>
              </div>

              <div class="flex gap-2">
                <Button class="!px-2.5 !py-1.5 !text-xs" onClick={() => resetPassword(user)}>
                  Set password
                </Button>
                <Button class="!px-2.5 !py-1.5 !text-xs" onClick={() => toggleAdmin(user)}>
                  {user.admin ? "Revoke admin" : "Make admin"}
                </Button>
                <Button
                  variant="ghost"
                  class="!px-2.5 !py-1.5 !text-xs"
                  disabled={user.username === currentUser}
                  title={
                    user.username === currentUser
                      ? "You cannot delete your own account"
                      : undefined
                  }
                  onClick={() => remove(user)}
                >
                  Delete
                </Button>
              </div>
            </div>

            {!user.admin && servers.length > 0 && (
              <div class="mt-4 space-y-2 border-t border-ink-700 pt-4">
                <p class="text-xs uppercase tracking-wider text-fg-muted">Server access</p>
                <div class="flex flex-wrap gap-3">
                  {servers.map((server) => (
                    <label key={server.id} class="flex items-center gap-2 text-sm">
                      <input
                        type="checkbox"
                        checked={user.servers.includes(server.id)}
                        onChange={(e) =>
                          setAccess(user, server.id, (e.target as HTMLInputElement).checked)
                        }
                        class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
                      />
                      {server.name}
                    </label>
                  ))}
                </div>
              </div>
            )}
          </Card>
        ))}
      </div>
    </div>
  );
}

function CreateUser({
  servers,
  onCreated,
}: {
  servers: Server[];
  onCreated: () => void;
}) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [admin, setAdmin] = useState(false);
  const [granted, setGranted] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(event: Event) {
    event.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await api.createUser({ username, password, admin, servers: granted });
      onCreated();
    } catch (e) {
      setError(e instanceof Error ? e.message : "could not create the account");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card title="New account">
      <form onSubmit={submit} class="space-y-4">
        {error && <Banner kind="error">{error}</Banner>}

        <div class="grid gap-4 sm:grid-cols-2">
          <Field label="Username">
            <Input
              value={username}
              autocomplete="off"
              onInput={(e) => setUsername((e.target as HTMLInputElement).value)}
            />
          </Field>
          <Field label="Password" hint="At least 8 characters.">
            <Input
              type="password"
              value={password}
              autocomplete="new-password"
              onInput={(e) => setPassword((e.target as HTMLInputElement).value)}
            />
          </Field>
        </div>

        <label class="flex items-center gap-2.5 text-sm text-fg-muted">
          <input
            type="checkbox"
            checked={admin}
            onChange={(e) => setAdmin((e.target as HTMLInputElement).checked)}
            class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
          />
          Administrator — full access to every server and account
        </label>

        {!admin && servers.length > 0 && (
          <div class="space-y-2">
            <p class="text-xs uppercase tracking-wider text-fg-muted">Server access</p>
            <div class="flex flex-wrap gap-3">
              {servers.map((server) => (
                <label key={server.id} class="flex items-center gap-2 text-sm">
                  <input
                    type="checkbox"
                    checked={granted.includes(server.id)}
                    onChange={(e) =>
                      setGranted((prev) =>
                        (e.target as HTMLInputElement).checked
                          ? [...prev, server.id]
                          : prev.filter((s) => s !== server.id),
                      )
                    }
                    class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
                  />
                  {server.name}
                </label>
              ))}
            </div>
          </div>
        )}

        <Button type="submit" variant="primary" disabled={busy}>
          {busy ? "Creating…" : "Create account"}
        </Button>
      </form>
    </Card>
  );
}
