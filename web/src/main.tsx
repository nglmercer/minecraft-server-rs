import { render } from "preact";
import { App } from "./app";
import { DialogProvider } from "./components/Modal";
import { ToastProvider } from "./components/Toast";
import { I18nProvider } from "./i18n";
import "./index.css";

const root = document.getElementById("app");

if (root) {
  render(
    <I18nProvider>
      <ToastProvider>
        <DialogProvider>
          <App />
        </DialogProvider>
      </ToastProvider>
    </I18nProvider>,
    root,
  );
}
