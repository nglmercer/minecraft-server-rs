import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import { Button } from "../components/ui";
import { useToast } from "../components/Toast";
import { useT } from "../i18n";
import type { HorizonSettings } from "../types";

const DEFAULTS: HorizonSettings = {
  online_mode: true,
  accept_older_clients: false,
  enforce_secure_profile: true,
};

/** The three Horizon settings that decide which Minecraft clients may join. */
export function HorizonClientSettings({ serverId }: { serverId: string }) {
  const t = useT();
  const toast = useToast();
  const [form, setForm] = useState<HorizonSettings>(DEFAULTS);
  const [busy, setBusy] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    let active = true;
    api
      .horizonSettings(serverId)
      .then((settings) => {
        if (active) {
          setForm(settings);
          setLoaded(true);
        }
      })
      .catch((error: unknown) => {
        if (active) toast.error(error instanceof Error ? error.message : String(error));
      });
    return () => {
      active = false;
    };
  }, [serverId]);

  async function save() {
    setBusy(true);
    try {
      const saved = await api.updateHorizonSettings(serverId, form);
      setForm(saved);
      toast.success(t("settings.saved"));
    } catch (error) {
      toast.error(error instanceof Error ? error.message : t("errors.generic"));
    } finally {
      setBusy(false);
    }
  }

  function row(key: keyof HorizonSettings, label: string, hint: string) {
    return (
      <label class="mt-3 flex items-start gap-2.5 border-t border-ink-700 pt-3 text-sm text-fg-muted sm:items-center">
        <input
          type="checkbox"
          checked={form[key]}
          disabled={!loaded}
          onChange={(event) =>
            setForm({ ...form, [key]: (event.target as HTMLInputElement).checked })
          }
          class="size-4 rounded border-ink-600 bg-ink-900 accent-[var(--color-accent)]"
        />
        <span>
          <span class="font-medium text-fg">{label}</span>
          <span class="block text-xs">{hint}</span>
        </span>
      </label>
    );
  }

  return (
    <div>
      {row("online_mode", t("settings.onlineMode"), t("settings.onlineModeHint"))}
      {row(
        "accept_older_clients",
        t("settings.acceptOlderClients"),
        t("settings.acceptOlderClientsHint"),
      )}
      {row(
        "enforce_secure_profile",
        t("settings.enforceSecureProfile"),
        t("settings.enforceSecureProfileHint"),
      )}
      <Button
        type="button"
        variant="primary"
        class="mt-4 w-full sm:w-auto"
        disabled={busy || !loaded}
        onClick={save}
      >
        {busy ? t("common.saving") : t("settings.saveChanges")}
      </Button>
    </div>
  );
}
