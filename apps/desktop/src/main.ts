import { createApp, ref } from "vue";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import "@fontsource-variable/source-sans-3";
import "@fontsource-variable/source-code-pro";
import "sit-onyx/style.css";
import "sit-onyx/global.css";
import "./styles.css";
import App from "./App.vue";
import { pairApi, safeError } from "./lib/ipc";
import type { SafeError, Settings } from "./lib/contracts";

let initialSettings: Settings | null = null;
let initialError: SafeError | null = null;
try {
  initialSettings = await pairApi.listPairs();
} catch (error) {
  initialError = safeError(error);
}

const app = createApp(App, { initialSettings, initialError });
app.use(createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } }));
app.mount("#app");
