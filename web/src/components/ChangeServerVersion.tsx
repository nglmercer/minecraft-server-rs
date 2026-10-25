import { useEffect, useState } from "preact/hooks";
import { api } from "../api";
import type { Server } from "../types";
import { recommendedJavaForVersion } from "../minecraftJava";
import { useT } from "../i18n";
import { Compatibility } from "./Compatibility";
import { Modal } from "./Modal";
import { Banner, Button, Field, Select } from "./ui";

export function ChangeServerVersion({ server, onClose, onSaved }: {
  server: Server;
  onClose: () => void;
  onSaved: () => void;
}) {
  const t = useT();
  const [core, setCore] = useState(server.core);
  const [version, setVersion] = useState(server.version);
  const [java, setJava] = useState(server.java_major);
  const [providers, setProviders] = useState<string[]>([]);
  const [catalog, setCatalog] = useState<{ core: string; versions: string[] } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [allowed, setAllowed] = useState(false);
  const [busy, setBusy] = useState(false);
  const stopped = server.status === "offline" || server.status === "crashed";
  useEffect(() => {
    let active = true;
    api.providers().then(values => {
      if (active) setProviders(values.filter(p => p.server).map(p => p.id));
    }).catch(e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, []);
  useEffect(() => {
    let active = true;
    setCatalog(null);
    setError(null);
    api.versions(core).then(versions => {
      if (active) setCatalog({ core, versions });
    }).catch(e => { if (active) setError(String(e)); });
    return () => { active = false; };
  }, [core]);
  const versions = catalog?.core === core ? catalog.versions : [];
  const valid = versions.includes(version);
  const changed = core !== server.core || version !== server.version || java !== server.java_major;
  async function save() {
    if (busy || !stopped || !allowed || !valid || !changed) return;
    setBusy(true);
    setError(null);
    try {
      await api.updateServer(server.id, { core, version, java_major: java });
      onSaved();
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally { setBusy(false); }
  }
  return (
    <Modal title={t("settings.changeVersion")} onClose={() => { if (!busy) onClose(); }} footer={<>
      <Button disabled={busy} onClick={onClose}>{t("common.cancel")}</Button>
      <Button variant="primary" disabled={busy || !stopped || !allowed || !valid || !changed} onClick={() => void save()}>
        {busy ? t("common.saving") : t("settings.saveChanges")}
      </Button>
    </>}>
      <div class="space-y-4">
        <Banner kind="info">{t("settings.changeVersionHelp")}</Banner>
        {error && <Banner kind="error">{error}</Banner>}
        <Field label={t("settings.coreType")}>
          <Select value={core} disabled={busy} onInput={e => {
            setCore(e.currentTarget.value); setVersion(""); setAllowed(false);
          }}>
            {[...new Set([server.core, ...providers])].map(p => <option key={p} value={p}>{p}</option>)}
          </Select>
        </Field>
        <Field label={t("settings.minecraftVersion")}>
          <Select value={version} disabled={busy || !catalog} onInput={e => {
            const next = e.currentTarget.value;
            setVersion(next); setJava(recommendedJavaForVersion(next)); setAllowed(false);
          }}>
            <option value="">{t("settings.selectVersion")}</option>
            {version && !valid && <option value={version}>{version}</option>}
            {versions.map(v => <option key={v} value={v}>{v}</option>)}
          </Select>
        </Field>
        <Field label={t("settings.javaVersion")}>
          <Select value={java} disabled={busy} onInput={e => setJava(Number(e.currentTarget.value))}>
            {[...new Set([8, 11, 17, 21, 25, java])].sort((a, b) => a - b).map(v => <option key={v} value={v}>Java {v}</option>)}
          </Select>
        </Field>
        <Compatibility core={core} version={version} javaMajor={java} onAllowedChange={setAllowed} />
      </div>
    </Modal>
  );
}
