import { defineConfig } from 'vite';
import { copyFileSync, cpSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import envCompatible from 'vite-plugin-env-compatible';
import { ViteMinifyPlugin } from 'vite-plugin-minify';
import ViteRestart from 'vite-plugin-restart'

export default defineConfig({
  base: '',
  server: {
    host: '0.0.0.0',
    allowedHosts: true,
  },
  plugins: [
    { name: 'ivnc-public-shell', closeBundle() {
      const root = fileURLToPath(new URL('.', import.meta.url));
      copyFileSync(root + 'sw.js', root + 'dist/sw.js');
      copyFileSync(root + 'manifest.json', root + 'dist/manifest.json');
      cpSync(root + 'icons', root + 'dist/icons', { recursive: true });
    } },
    envCompatible(),
    ViteMinifyPlugin(),
    ViteRestart({restart: ['ivnc-core.js', 'lib/**','ivnc-version.txt']}),
  ],
  build: {
    target: 'chrome94',
    rollupOptions: {
      input: {
        main: './index.html',
        console: './console.html',
        'ivnc-core': './ivnc-core.js',
        login: './login.html',
        'auth-client': './auth-client.js',
      },
      output: {
        entryFileNames: (chunkInfo) => {
          return chunkInfo.name === 'ivnc-core' ? 'ivnc-core.js' : '[name].js';
        }
      }
    }
  },
})
