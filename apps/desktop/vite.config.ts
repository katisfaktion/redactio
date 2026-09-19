import vue from '@vitejs/plugin-vue'
import { defineConfig } from 'vitest/config'

// https://vite.dev/config/
export default defineConfig({
  plugins: [vue()],
  server: {
    port: 1420,
    strictPort: true,
  },
  test: {
    environment: 'jsdom',
  },
})
