import { useRef } from "preact/hooks";
import { api } from "./api";
import { useDialogs } from "./components/Modal";
import { useT } from "./i18n";
import type { Server } from "./types";

/** One guarded launch flow shared by the dashboard and server detail. */
export function useServerPower() {
  const dialogs = useDialogs();
  const t = useT();
  const pending = useRef(new Set<string>());
  return async (
    server: Server,
    action: "start" | "stop" | "restart" | "kill",
  ) => {
    if (pending.current.has(server.id)) return null;
    pending.current.add(server.id);
    try {
      // Refresh before launch: another tab may have changed the acceptance.
      if (action === "start" || action === "restart") {
        const current = await api.server(server.id);
        if (!current.eula_accepted) {
          const accepted = await dialogs.confirm({
            title: t("launch.eulaTitle"),
            body: (
              <div class="space-y-3">
                <p>{t("launch.eulaBody")}</p>
                <a
                  class="text-accent underline"
                  href="https://www.minecraft.net/eula"
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {t("createServer.eulaLink")}
                </a>
              </div>
            ),
            confirmLabel: t("launch.acceptAndStart"),
          });
          if (!accepted) return null;
          await api.updateServer(server.id, { eula_accepted: true });
        }
      }
      return await api.power(server.id, action);
    } finally {
      pending.current.delete(server.id);
    }
  };
}
