interface AppFooterProps {
  brandText: string;
  tone: 'cloud' | 'arena';
  githubUrl?: string;
  docsUrl?: string;
}

const TONE_STYLES: Record<AppFooterProps['tone'], { border: string; text: string }> = {
  cloud: {
    border: 'border-cloud-elements-dividerColor',
    text: 'text-cloud-elements-textTertiary',
  },
  arena: {
    border: 'border-arena-elements-dividerColor',
    text: 'text-arena-elements-textTertiary',
  },
};

export function AppFooter({
  brandText,
  tone,
  githubUrl = 'https://github.com/tangle-network',
  docsUrl = 'https://docs.tangle.tools',
}: AppFooterProps) {
  const styles = TONE_STYLES[tone];

  return (
    <footer className={`border-t py-5 mt-12 ${styles.border}`}>
      <div className={`mx-auto max-w-7xl px-4 sm:px-6 flex items-center justify-between text-xs ${styles.text}`}>
        <span className="font-data">{brandText}</span>
        <div className="flex items-center gap-5">
          <a href={githubUrl} target="_blank" rel="noopener noreferrer" className="hover:text-violet-700 dark:hover:text-violet-400 transition-colors">GitHub</a>
          <a href={docsUrl} target="_blank" rel="noopener noreferrer" className="hover:text-violet-700 dark:hover:text-violet-400 transition-colors">Docs</a>
        </div>
      </div>
    </footer>
  );
}
