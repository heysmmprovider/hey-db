import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
export default defineConfig({ plugins: [react()], server: { port: 1420, strictPort: true }, clearScreen: false, envPrefix: ['VITE_'], build: { target: 'es2022', rollupOptions: { output: { manualChunks(id) { if (id.includes('@codemirror') || id.includes('@lezer')) return 'sql-editor'; } } } } });
