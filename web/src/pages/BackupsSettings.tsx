import { BackupStorage } from "../components/BackupStorage";
import { useT } from "../i18n";

export function BackupsSettings() {
  const t = useT();
  return (
    <div class="mx-auto w-full max-w-3xl space-y-6 px-4 py-8 sm:px-6">
      <header>
        <h1 class="text-2xl font-semibold">{t("backups.storageTitle")}</h1>
        <p class="mt-1 text-sm text-fg-muted">{t("backups.storageSubtitle")}</p>
      </header>
      <BackupStorage />
    </div>
  );
}
