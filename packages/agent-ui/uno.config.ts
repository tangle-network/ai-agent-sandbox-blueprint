import { defineConfig, presetWind4, presetIcons } from 'unocss';

export default defineConfig({
  presets: [
    presetWind4(),
    presetIcons({
      collections: {
        ph: () => import('@iconify-json/ph/icons.json').then((m) => m.default),
      },
    }),
  ],
  safelist: [
    // Status icons
    'i-ph:check-circle', 'i-ph:circle-notch', 'i-ph:warning-circle', 'i-ph:x-circle',
    'i-ph:caret-right', 'i-ph:caret-down', 'i-ph:caret-up',
    // Tool category icons
    'i-ph:terminal-window', 'i-ph:note-pencil', 'i-ph:file-magnifying-glass',
    'i-ph:magnifying-glass', 'i-ph:pencil-simple-line', 'i-ph:robot',
    'i-ph:globe-hemisphere-west', 'i-ph:clipboard-text', 'i-ph:gear',
    // Chat icons
    'i-ph:brain', 'i-ph:copy', 'i-ph:check', 'i-ph:code',
    'i-ph:arrow-down', 'i-ph:arrow-right', 'i-ph:arrow-left',
    'i-ph:paper-plane-tilt', 'i-ph:spinner',
    'i-ph:git-diff', 'i-ph:list-bullets', 'i-ph:lightning',
  ],
});
