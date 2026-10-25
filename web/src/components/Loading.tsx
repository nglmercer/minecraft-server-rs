import { useT } from "../i18n";

export function Loading() {
  const t = useT();
  return <div role="status" class="p-6 text-sm text-fg-muted">{t("common.loading")}</div>;
}
