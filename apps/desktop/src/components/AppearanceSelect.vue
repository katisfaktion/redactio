<script setup lang="ts">
import { OnyxSelect } from "sit-onyx";
import { onUnmounted, ref, watch } from "vue";

const options = [
  { value: "dark", label: "Dunkel" },
  { value: "light", label: "Hell" },
  { value: "auto", label: "Wie Windows" },
];
const appearance = ref("dark");
try {
  const saved = localStorage.getItem("redactio-appearance");
  if (options.some(option => option.value === saved)) appearance.value = saved!;
} catch { /* Appearance remains usable when preference storage is unavailable. */ }
const system = window.matchMedia?.("(prefers-color-scheme: dark)");
const systemDark = ref(system?.matches ?? false);
function changed() { systemDark.value = system?.matches ?? false; }
system?.addEventListener("change", changed);
onUnmounted(() => system?.removeEventListener("change", changed));
watch([appearance, systemDark], () => {
  document.documentElement.classList.toggle("dark", appearance.value === "dark" || (appearance.value === "auto" && systemDark.value));
}, { immediate: true });
watch(appearance, value => {
  try { localStorage.setItem("redactio-appearance", value); }
  catch { /* Keep the current selection for this session. */ }
});
</script>

<template>
  <OnyxSelect v-model="appearance" data-testid="appearance-select" spellcheck="false" label="Erscheinungsbild" list-label="Farbschema"
    :options="options" :hide-clear-icon="true" />
</template>
