import { createApp, ref } from "vue";
import { createOnyx } from "sit-onyx";
import onyxDeDE from "sit-onyx/locales/de-DE.json";
import "@fontsource-variable/source-sans-3";
import "@fontsource-variable/source-code-pro";
import "sit-onyx/style.css";
import "sit-onyx/global.css";
import "./styles.css";
import App from "./App.vue";
import type { Settings } from "./lib/contracts";

const initialSettings: Settings = {
  schema_version: 1,
  sync_pairs: [],
  selected_sync_pair_id: null,
};
const app = createApp(App, { initialSettings });
app.use(createOnyx({ i18n: { locale: ref("de-DE"), messages: { "de-DE": onyxDeDE } } }));
app.mount("#app");
