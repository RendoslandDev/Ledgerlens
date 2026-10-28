import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  // `@ledgerlens/bindings` is a linked workspace package; resolve its imports
  // (`@tauri-apps/api`) from this app's node_modules.
  resolve: { preserveSymlinks: true },
  // Serve the generated bindings from source: pre-bundling caches them, so a dev server kept
  // running across `npm run bindings` would otherwise serve the old commands.
  optimizeDeps: {
    exclude: ['@ledgerlens/bindings'],
    include: ['@tauri-apps/api/core'],
  },
  clearScreen: false,
  server: {
    strictPort: true,
    watch: { ignored: ['**/src-tauri/**'] },
  },
});