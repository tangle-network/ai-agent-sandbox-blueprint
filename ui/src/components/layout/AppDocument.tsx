import type { ReactNode } from 'react';
import { Links, Meta, Scripts, ScrollRestoration } from 'react-router';

interface AppDocumentProps {
  children: ReactNode;
  description: string;
  themeStorageKeys: [string, string];
}

function buildInlineThemeCode(themeStorageKeys: [string, string]): string {
  const [primaryKey, secondaryKey] = themeStorageKeys;
  return `
    (function() {
      var theme = localStorage.getItem('${primaryKey}') || localStorage.getItem('${secondaryKey}');
      if (!theme) {
        theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
      }
      document.querySelector('html').setAttribute('data-theme', theme);
    })();
  `;
}

export function AppDocument({ children, description, themeStorageKeys }: AppDocumentProps) {
  return (
    <html lang="en" data-theme="dark">
      <head>
        <meta charSet="utf-8" />
        <meta name="viewport" content="width=device-width, initial-scale=1" />
        <meta name="description" content={description} />
        <Meta />
        <Links />
        <link rel="icon" href="/favicon.svg" type="image/svg+xml" />
        <link rel="preconnect" href="https://fonts.googleapis.com" />
        <link rel="preconnect" href="https://fonts.gstatic.com" crossOrigin="anonymous" />
        <link
          rel="stylesheet"
          href="https://fonts.googleapis.com/css2?family=DM+Sans:opsz,wght@9..40,400;500;600;700&family=IBM+Plex+Mono:wght@400;500;600;700&family=Outfit:wght@400;500;600;700;800;900&display=swap"
        />
        <script dangerouslySetInnerHTML={{ __html: buildInlineThemeCode(themeStorageKeys) }} />
      </head>
      <body>
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}
