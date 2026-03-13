import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import cssInjectedByJsPlugin from 'vite-plugin-css-injected-by-js';

export default defineConfig({
  plugins: [
    react(),
    cssInjectedByJsPlugin(), // Inlines CSS into JS — one file, zero config for consumers
  ],
  define: {
    'process.env.NODE_ENV': JSON.stringify('production'),
  },
  build: {
    lib: {
      entry: 'src/index.ts',
      name: 'LinggenUI',
      formats: ['umd', 'es'],
      fileName: (format) => `linggen-ui.${format}.js`,
    },
    rollupOptions: {
      output: {
        exports: 'named',
      },
    },
    cssCodeSplit: false,
    sourcemap: true,
    minify: false,
  },
});
