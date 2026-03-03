import { Toaster } from 'sonner';
import { useThemeValue } from '@tangle/blueprint-ui';

interface AppToasterProps {
  tone: 'cloud' | 'arena';
}

const TONE_TEXT_COLOR: Record<AppToasterProps['tone'], string> = {
  cloud: 'var(--cloud-elements-textPrimary)',
  arena: 'var(--arena-elements-textPrimary)',
};

export function AppToaster({ tone }: AppToasterProps) {
  const theme = useThemeValue();

  return (
    <Toaster
      position="bottom-right"
      theme={theme as 'light' | 'dark' | 'system'}
      richColors
      closeButton
      duration={3000}
      toastOptions={{
        style: {
          background: 'var(--glass-bg-strong)',
          backdropFilter: 'blur(16px)',
          border: '1px solid var(--glass-border)',
          color: TONE_TEXT_COLOR[tone],
        },
      }}
    />
  );
}
