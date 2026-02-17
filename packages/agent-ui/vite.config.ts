import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import dts from 'vite-plugin-dts';
import UnoCSS from 'unocss/vite';
import { resolve } from 'path';

export default defineConfig({
  plugins: [
    react(),
    UnoCSS(),
    dts({ rollupTypes: true }),
  ],
  resolve: {
    alias: { '~': resolve(__dirname, 'src') },
  },
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      formats: ['es'],
      fileName: 'index',
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
      ],
    },
  },
});
