import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import fs from 'fs';
import path from 'path';

// Helper to get port from linggen-agent.toml
function getBackendPort() {
  try {
    const tomlPath = path.resolve(__dirname, '../linggen-agent.toml');
    const content = fs.readFileSync(tomlPath, 'utf-8');
    const match = content.match(/\[server\][\s\S]*?port\s*=\s*(\d+)/);
    return match ? match[1] : '8080';
  } catch (e) {
    return '8080';
  }
}

const backendPort = getBackendPort();

export default defineConfig({
  plugins: [
    react(),
    tailwindcss(),
  ],
  server: {
    proxy: {
      '/api': `http://localhost:${backendPort}`,
    },
  },
  build: {
    outDir: 'dist',
    emptyOutDir: true,
  },
});
