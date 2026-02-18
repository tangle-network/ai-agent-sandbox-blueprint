import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import dts from 'vite-plugin-dts';
import UnoCSS from 'unocss/vite';
import { resolve } from 'path';

export default defineConfig({
  plugins: [
    react(),
    UnoCSS(),
    dts(),
  ],
  resolve: {
    alias: { '~': resolve(__dirname, 'src') },
  },
  build: {
    lib: {
      entry: {
        index: resolve(__dirname, 'src/index.ts'),
        terminal: resolve(__dirname, 'src/terminal.ts'),
      },
      formats: ['es'],
    },
    rollupOptions: {
      external: [
        'react',
        'react-dom',
        'react/jsx-runtime',
        'nanostores',
        '@nanostores/react',
        '@radix-ui/react-collapsible',
        'framer-motion',
        '@tanstack/react-query',
        '@xterm/xterm',
        '@xterm/addon-fit',
        '@xterm/addon-web-links',
      ],
    },
  },
});
