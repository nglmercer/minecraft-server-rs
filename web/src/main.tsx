import { render } from "preact";
import { App } from "./app";
import { MenuProvider } from "./components/Menu";
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
          <MenuProvider>
            <App />
          </MenuProvider>
        </DialogProvider>
      </ToastProvider>
    </I18nProvider>,
    root,
  );
}
