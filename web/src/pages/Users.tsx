import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Button, Card, Field, Input } from "../components/ui";
import * as Icon from "../components/icons";
import { Modal, useDialogs } from "../components/Modal";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import type { PanelUser, Server } from "../types";

export function Users({ currentUser }: { currentUser: string }) {
  const t = useT();
  const toast = useToast();
  const dialogs = useDialogs();

  const [users, setUsers] = useState<PanelUser[]>([]);
  const [servers, setServers] = useState<Server[]>([]);
  const [creating, setCreating] = useState(false);

  async function refresh() {
    try {
      const [list, allServers] = await Promise.all([api.users(), api.servers()]);
      setUsers(list);
      setServers(allServers);
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.loadAccounts"));
    }
  }

  useEffect(() => {
    void refresh();
  }, []);

  const fail = (error: unknown) =>
    toast.error(error instanceof Error ? error.message : t("errors.actionFailed"));

  async function remove(user: PanelUser) {
    const confirmed = await dialogs.confirm({
      title: t("users.deleteTitle", { name: user.username }),
      body: t("users.deleteBody"),
      confirmLabel: t("common.delete"),
      danger: true,
    });
    if (!confirmed) return;

    try {
      await api.deleteUser(user.username);
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function toggleAdmin(user: PanelUser) {
    try {
      await api.updateUser(user.username, { admin: !user.admin });
      toast.success(
        t("users.roleChanged", {
          name: user.username,
          role: user.admin ? t("users.roleUser") : t("users.roleAdmin"),
        }),
      );
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function resetPassword(user: PanelUser) {
    const password = await dialogs.prompt({
      title: t("users.setPasswordTitle", { name: user.username }),
      label: t("users.setPasswordLabel"),
      hint: t("users.passwordHint"),
      password: true,
      confirmLabel: t("common.save"),
    });
    if (!password) return;

    try {
      await api.updateUser(user.username, { password });
      toast.success(t("users.passwordChanged", { name: user.username }));
    } catch (e) {
      fail(e);
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
      fail(e);
    }
  }

  return (
    <div class="mx-auto w-full max-w-4xl space-y-6 px-4 py-8 sm:px-6">
      <header class="flex items-end justify-between gap-4">
        <div>
          <h1 class="text-2xl font-semibold">{t("users.title")}</h1>
          <p class="text-sm text-fg-muted">{t("users.count", { count: users.length })}</p>
        </div>
        <Button
          variant="primary"
          icon={<Icon.Plus size={15} />}
          onClick={() => setCreating(true)}
        >
          {t("users.newAccount")}
        </Button>
      </header>

      {creating && (
        <CreateUser
          servers={servers}
          onClose={() => setCreating(false)}
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
                    <span class="ml-2 text-xs text-fg-muted">{t("users.you")}</span>
                  )}
                  {user.admin && (
                    <span class="ml-2 text-xs text-accent">{t("nav.admin")}</span>
                  )}
                </p>
                <p class="mt-0.5 text-xs text-fg-muted">
                  {user.admin
                    ? t("users.adminAll")
                    : t("users.grantedCount", { count: user.servers.length })}
                </p>
              </div>

              <div class="flex flex-wrap items-center gap-2">
                <Button class="!px-2.5 !py-1.5 !text-xs" onClick={() => resetPassword(user)}>
                  {t("users.setPassword")}
                </Button>
                <Button class="!px-2.5 !py-1.5 !text-xs" onClick={() => toggleAdmin(user)}>
                  {user.admin ? t("users.revokeAdmin") : t("users.makeAdmin")}
                </Button>
                <Button
                  variant="ghost"
                  class="!px-2.5 !py-1.5 !text-xs"
                  disabled={user.username === currentUser}
                  title={
                    user.username === currentUser ? t("users.cannotDeleteSelf") : undefined
                  }
                  onClick={() => remove(user)}
                >
                  {t("common.delete")}
                </Button>
              </div>
            </div>

            {!user.admin && servers.length > 0 && (
              <div class="mt-4 space-y-2 border-t border-ink-700 pt-4">
                <p class="text-xs uppercase tracking-wider text-fg-muted">{t("users.serverAccess")}</p>
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
  onClose,
  onCreated,
}: {
  servers: Server[];
  onClose: () => void;
  onCreated: () => void;
}) {
  const t = useT();
  const toast = useToast();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [admin, setAdmin] = useState(false);
  const [granted, setGranted] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);

  async function submit(event?: Event) {
    event?.preventDefault();
    if (busy) return;
    setBusy(true);
    try {
      await api.createUser({ username, password, admin, servers: granted });
      onCreated();
    } catch (e) {
      toast.error(e instanceof Error ? e.message : t("errors.generic"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Modal
      title={t("users.newAccount")}
      onClose={onClose}
      width="lg"
      footer={
        <>
          <Button variant="ghost" onClick={onClose} disabled={busy}>
            {t("common.cancel")}
          </Button>
          <Button
            type="submit"
            form="create-user-form"
            variant="primary"
            disabled={busy}
            onClick={submit}
          >
            {busy ? t("common.creating") : t("users.createAccount")}
          </Button>
        </>
      }
    >
      <form id="create-user-form" onSubmit={submit} class="space-y-4">
        <div class="grid gap-4 sm:grid-cols-2">
          <Field label={t("users.username")}>
            <Input
              value={username}
              autocomplete="off"
              onInput={(e) => setUsername((e.target as HTMLInputElement).value)}
            />
          </Field>
          <Field label={t("users.password")} hint={t("users.passwordHint")}>
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
          {t("users.adminLabel")}
        </label>

        {!admin && servers.length > 0 && (
          <div class="space-y-2 pt-1">
            <p class="text-xs uppercase tracking-wider text-fg-muted">{t("users.serverAccess")}</p>
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
      </form>
    </Modal>
  );
}
