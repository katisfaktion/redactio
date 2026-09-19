<script setup lang="ts">
import { OnyxAppLayout, OnyxButton, OnyxPageLayout } from "sit-onyx";

import type { Settings } from "./lib/contracts";

defineProps<{
  initialSettings: Settings;
}>();
</script>

<template>
  <OnyxAppLayout>
    <OnyxPageLayout>
      <main class="shell">
        <header>
          <p class="eyebrow">Redactio</p>
          <h1>Dokumente sicher schwärzen</h1>
        </header>

        <section v-if="initialSettings.sync_pairs.length === 0" class="setup" aria-labelledby="setup-heading">
          <div>
            <h2 id="setup-heading">Noch kein Ordnerpaar eingerichtet</h2>
            <p>Wählen Sie einen Quellordner und einen getrennten Zielordner aus.</p>
          </div>
          <OnyxButton label="Ordnerpaar hinzufügen" />
        </section>
      </main>
    </OnyxPageLayout>
  </OnyxAppLayout>
</template>

<style scoped>
.shell {
  display: grid;
  gap: var(--onyx-spacing-2xl);
  min-height: 100%;
  padding-block: var(--onyx-spacing-2xl);
}

.eyebrow {
  color: var(--onyx-color-text-icons-neutral-intense);
  font-weight: var(--onyx-font-weight-semibold);
  margin: 0 0 var(--onyx-spacing-xs);
}

h1,
h2,
p {
  margin-top: 0;
}

.setup {
  align-items: center;
  background: var(--onyx-color-base-background-blank);
  border: var(--onyx-1px-in-rem) solid var(--onyx-color-component-border-neutral);
  border-radius: var(--onyx-radius-md);
  display: flex;
  flex-wrap: wrap;
  gap: var(--onyx-spacing-xl);
  justify-content: space-between;
  padding: var(--onyx-spacing-xl);
}

.setup p {
  margin-bottom: 0;
}
</style>
