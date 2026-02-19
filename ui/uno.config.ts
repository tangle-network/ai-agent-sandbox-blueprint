import { icons as phIcons } from '@iconify-json/ph';
import { defineConfig, presetIcons, transformerDirectives } from 'unocss';
import { bpThemeTokens } from '@tangle/blueprint-ui/preset';
import { presetAnimations } from 'unocss-preset-animations';
import { presetWind4 } from 'unocss/preset-wind4';

/*
 * TANGLE CLOUD — Design System
 * Infrastructure-grade UI with glass morphism.
 * Deep obsidian base, teal for running, amber for stopped, violet for accents.
 */

const BASE_COLORS = {
  white: '#F0F0F5',
  black: '#0A0A0F',
  obsidian: {
    50: '#E8E8ED',
    100: '#C8C8D0',
    200: '#9898A5',
    300: '#6A6A7A',
    400: '#4A4A5C',
    500: '#35354A',
    600: '#2A2A3A',
    700: '#1E1E2C',
    800: '#15151F',
    900: '#0E0E16',
    950: '#0A0A0F',
  },
  teal: {
    50: '#E6FFFA',
    100: '#B2F5EA',
    200: '#81E6D9',
    300: '#4FD1C5',
    400: '#38B2AC',
    500: '#319795',
    600: '#2C7A7B',
    700: '#285E61',
    800: '#234E52',
    900: '#1D4044',
    950: '#0F2A2D',
  },
  emerald: {
    50: '#E8FFF3',
    100: '#C0FFE0',
    200: '#80FFBC',
    300: '#40FF99',
    400: '#00FF88',
    500: '#00D573',
    600: '#00AA5C',
    700: '#008048',
    800: '#005530',
    900: '#002B18',
    950: '#00150C',
  },
  crimson: {
    50: '#FFF0F2',
    100: '#FFD6DC',
    200: '#FFB0BC',
    300: '#FF8A9C',
    400: '#FF4D6A',
    500: '#FF3B5C',
    600: '#E5223E',
    700: '#B91C32',
    800: '#8C1525',
    900: '#5F0E18',
    950: '#33070D',
  },
  amber: {
    50: '#FFFBEB',
    100: '#FFF3C4',
    200: '#FFE68A',
    300: '#FFD54F',
    400: '#FFC107',
    500: '#FFB800',
    600: '#E5A500',
    700: '#B88000',
    800: '#8C6200',
    900: '#5F4200',
    950: '#332300',
  },
  blue: {
    50: '#EBF5FF',
    100: '#D6EBFF',
    200: '#B0D9FF',
    300: '#80C4FF',
    400: '#4AABFF',
    500: '#00B4FF',
    600: '#008FCC',
    700: '#006B99',
    800: '#004766',
    900: '#002433',
    950: '#00121A',
  },
  violet: {
    50: '#F5F0FF',
    100: '#E8DBFF',
    200: '#D1B8FF',
    300: '#B990FF',
    400: '#A370FF',
    500: '#8B5CF6',
    600: '#7040DC',
    700: '#5628B8',
    800: '#3F1D8C',
    900: '#2A1260',
    950: '#150933',
  },
} as const;

const COLOR_PRIMITIVES = {
  ...BASE_COLORS,
  alpha: {
    white: generateAlphaPalette('#F0F0F5'),
    black: generateAlphaPalette('#0A0A0F'),
    teal: generateAlphaPalette('#38B2AC'),
    emerald: generateAlphaPalette('#00FF88'),
    crimson: generateAlphaPalette('#FF3B5C'),
    amber: generateAlphaPalette('#FFB800'),
    violet: generateAlphaPalette('#8B5CF6'),
    blue: generateAlphaPalette('#00B4FF'),
  },
} as const;

const SHADCN_COLORS = {
  background: 'hsl(var(--background))',
  foreground: 'hsl(var(--foreground))',
  card: {
    DEFAULT: 'hsl(var(--card))',
    foreground: 'hsl(var(--card-foreground))',
  },
  popover: {
    DEFAULT: 'hsl(var(--popover))',
    foreground: 'hsl(var(--popover-foreground))',
  },
  primary: {
    DEFAULT: 'hsl(var(--primary))',
    foreground: 'hsl(var(--primary-foreground))',
  },
  secondary: {
    DEFAULT: 'hsl(var(--secondary))',
    foreground: 'hsl(var(--secondary-foreground))',
  },
  muted: {
    DEFAULT: 'hsl(var(--muted))',
    foreground: 'hsl(var(--muted-foreground))',
  },
  accent: {
    DEFAULT: 'hsl(var(--accent))',
    foreground: 'hsl(var(--accent-foreground))',
  },
  destructive: {
    DEFAULT: 'hsl(var(--destructive))',
    foreground: 'hsl(var(--destructive-foreground))',
  },
  border: 'hsl(var(--border))',
  input: 'hsl(var(--input))',
  ring: 'hsl(var(--ring))',
} as const;

export default defineConfig({
  content: {
    pipeline: {
      include: [/\.(tsx?|jsx?)$/, '../../blueprint-ui/src/**/*.{ts,tsx}', '../packages/agent-ui/src/**/*.{ts,tsx}'],
    },
  },
  shortcuts: {
    'cloud-ease': 'ease-[cubic-bezier(0.4,0,0.2,1)]',
    'transition-theme': 'transition-[background-color,border-color,color] duration-150 cloud-ease',
    'glass': 'bg-[var(--glass-bg)] backdrop-blur-xl border border-[var(--glass-border)]',
    'glass-hover': 'hover:border-[var(--glass-border-hover)] hover:bg-[var(--glass-bg-strong)]',
    'glass-strong': 'bg-[var(--glass-bg-strong)] backdrop-blur-2xl border border-[var(--glass-border)]',
    'text-glow-teal': 'text-teal-400 drop-shadow-[0_0_8px_rgba(56,178,172,0.4)]',
    'text-glow-emerald': 'text-emerald-400 drop-shadow-[0_0_8px_rgba(0,255,136,0.4)]',
    'text-glow-crimson': 'text-crimson-400 drop-shadow-[0_0_8px_rgba(255,59,92,0.4)]',
    'text-glow-amber': 'text-amber-400 drop-shadow-[0_0_8px_rgba(255,184,0,0.4)]',
    'glow-border-teal': 'shadow-[0_0_15px_rgba(56,178,172,0.15),inset_0_1px_0_rgba(255,255,255,0.05)]',
    'glow-border-emerald': 'shadow-[0_0_15px_rgba(0,255,136,0.15),inset_0_1px_0_rgba(255,255,255,0.05)]',
    'glow-border-crimson': 'shadow-[0_0_15px_rgba(255,59,92,0.15),inset_0_1px_0_rgba(255,255,255,0.05)]',
    'glow-border-amber': 'shadow-[0_0_15px_rgba(255,184,0,0.15),inset_0_1px_0_rgba(255,255,255,0.05)]',
  },
  rules: [
    ['b', {}],
    [/^font-display$/, () => ({ 'font-family': "'Outfit', system-ui, sans-serif" })],
    [/^font-body$/, () => ({ 'font-family': "'DM Sans', system-ui, sans-serif" })],
    [/^font-data$/, () => ({ 'font-family': "'IBM Plex Mono', 'JetBrains Mono', monospace" })],
    [/^noise-bg$/, () => ({
      'background-image': "url(\"data:image/svg+xml,%3Csvg viewBox='0 0 256 256' xmlns='http://www.w3.org/2000/svg'%3E%3Cfilter id='n'%3E%3CfeTurbulence type='fractalNoise' baseFrequency='0.9' numOctaves='4' stitchTiles='stitch'/%3E%3C/filter%3E%3Crect width='100%25' height='100%25' filter='url(%23n)' opacity='0.03'/%3E%3C/svg%3E\")",
    })],
  ],
  theme: {
    animation: {
      keyframes: {
        'fade-in': '{0%{opacity:0;transform:translateY(10px)}100%{opacity:1;transform:translateY(0)}}',
        'fade-in-up': '{0%{opacity:0;transform:translateY(20px)}100%{opacity:1;transform:translateY(0)}}',
        'glow-pulse': '{0%,100%{opacity:1}50%{opacity:0.6}}',
        'shimmer': '{0%{transform:translateX(-100%)}100%{transform:translateX(100%)}}',
        'status-pulse': '{0%,100%{opacity:1;transform:scale(1)}50%{opacity:0.6;transform:scale(0.95)}}',
      },
      durations: {
        'glow-pulse': '2s',
        'shimmer': '2s',
        'status-pulse': '2s',
      },
      timingFns: {
        'glow-pulse': 'ease-in-out',
        'shimmer': 'ease-in-out',
        'status-pulse': 'ease-in-out',
      },
      counts: {
        'glow-pulse': 'infinite',
        'shimmer': 'infinite',
        'status-pulse': 'infinite',
      },
    },
    colors: {
      ...COLOR_PRIMITIVES,
      ...SHADCN_COLORS,
      bp: bpThemeTokens('cloud'),
      cloud: {
        elements: {
          borderColor: 'var(--cloud-elements-borderColor)',
          borderColorActive: 'var(--cloud-elements-borderColorActive)',
          background: {
            depth: {
              1: 'var(--cloud-elements-bg-depth-1)',
              2: 'var(--cloud-elements-bg-depth-2)',
              3: 'var(--cloud-elements-bg-depth-3)',
              4: 'var(--cloud-elements-bg-depth-4)',
            },
          },
          textPrimary: 'var(--cloud-elements-textPrimary)',
          textSecondary: 'var(--cloud-elements-textSecondary)',
          textTertiary: 'var(--cloud-elements-textTertiary)',
          button: {
            primary: {
              background: 'var(--cloud-elements-button-primary-background)',
              backgroundHover: 'var(--cloud-elements-button-primary-backgroundHover)',
              text: 'var(--cloud-elements-button-primary-text)',
            },
            secondary: {
              background: 'var(--cloud-elements-button-secondary-background)',
              backgroundHover: 'var(--cloud-elements-button-secondary-backgroundHover)',
              text: 'var(--cloud-elements-button-secondary-text)',
            },
            danger: {
              background: 'var(--cloud-elements-button-danger-background)',
              backgroundHover: 'var(--cloud-elements-button-danger-backgroundHover)',
              text: 'var(--cloud-elements-button-danger-text)',
            },
          },
          icon: {
            success: 'var(--cloud-elements-icon-success)',
            error: 'var(--cloud-elements-icon-error)',
            warning: 'var(--cloud-elements-icon-warning)',
            primary: 'var(--cloud-elements-icon-primary)',
            secondary: 'var(--cloud-elements-icon-secondary)',
          },
          dividerColor: 'var(--cloud-elements-dividerColor)',
          item: {
            backgroundHover: 'var(--cloud-elements-item-backgroundHover)',
            backgroundActive: 'var(--cloud-elements-item-backgroundActive)',
          },
          focus: 'var(--cloud-elements-focus)',
        },
      },
    },
  },
  transformers: [transformerDirectives()],
  presets: [
    presetWind4({
      dark: {
        light: '[data-theme="light"]',
        dark: '[data-theme="dark"]',
      },
    }),
    presetAnimations(),
    presetIcons({
      warn: true,
      collections: {
        ph: () => phIcons,
      },
    }),
  ],
  safelist: [
    'i-ph:terminal',
    'i-ph:cloud',
    'i-ph:cpu',
    'i-ph:hard-drives',
    'i-ph:play',
    'i-ph:stop',
    'i-ph:arrow-clockwise',
    'i-ph:trash',
    'i-ph:camera',
    'i-ph:key',
    'i-ph:robot',
    'i-ph:gear',
    'i-ph:chart-bar',
    'i-ph:wallet',
    'i-ph:plus',
    'i-ph:sun',
    'i-ph:moon',
    'i-ph:caret-down',
    'i-ph:magnifying-glass',
    'i-ph:copy',
    'i-ph:sign-out',
    'i-ph:swap',
    'i-ph:plus-circle',
    'i-ph:check-bold',
    'i-ph:check-circle-fill',
    'i-ph:x-circle-fill',
    'i-ph:receipt',
    'i-ph:clock-counter-clockwise',
    'i-ph:desktop',
    'i-ph:flask',
    'i-ph:globe-hemisphere-west',
    'i-ph:globe',
    'i-ph:lightning',
    'i-ph:database',
    'i-ph:timer',
    'i-ph:shield-check',
    'i-ph:flow-arrow',
    'i-ph:list-dashes',
    'i-ph:pulse',
    'i-ph:circle-fill',
    'i-ph:info',
    'i-ph:caret-right',
    'i-ph:x',
    'i-ph:rocket-launch',
  ],
});

function generateAlphaPalette(hex: string) {
  return [1, 2, 3, 4, 5, 8, 10, 15, 20, 30, 40, 50, 60, 70, 80, 90, 100].reduce(
    (acc, opacity) => {
      const alpha = Math.round((opacity / 100) * 255)
        .toString(16)
        .padStart(2, '0');
      acc[opacity] = `${hex}${alpha}`;
      return acc;
    },
    {} as Record<number, string>,
  );
}
