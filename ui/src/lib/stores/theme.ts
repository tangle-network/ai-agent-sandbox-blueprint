import { atom } from 'nanostores';

export type Theme = 'dark' | 'light';

export const kTheme = 'sandbox_cloud_theme';
export const DEFAULT_THEME = 'dark';

export const themeStore = atom<Theme>(initStore());

function initStore() {
  if (typeof window !== 'undefined') {
    const persisted = localStorage.getItem(kTheme) as Theme | undefined;
    const attr = document.querySelector('html')?.getAttribute('data-theme');
    return persisted ?? (attr as Theme) ?? DEFAULT_THEME;
  }
  return DEFAULT_THEME;
}

export function toggleTheme() {
  const next = themeStore.get() === 'dark' ? 'light' : 'dark';
  themeStore.set(next);
  localStorage.setItem(kTheme, next);
  document.querySelector('html')?.setAttribute('data-theme', next);
}
