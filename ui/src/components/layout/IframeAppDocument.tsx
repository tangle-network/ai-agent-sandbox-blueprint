import type { ReactNode } from 'react';
import { Links, Meta, Scripts, ScrollRestoration } from 'react-router';
import { buildInlineThemeBootstrap } from '~/lib/theme';

interface IframeAppDocumentProps {
  children: ReactNode;
  description: string;
}

// Local replacement for blueprint-ui's `AppDocument`. The only behavioral
// difference is the inline bootstrap script: this one honors the parent
// shell's `?theme=light|dark` URL param so the iframe's first paint matches
// the embedding dapp (instead of always defaulting to dark and showing a
// black rectangle on a light-themed parent).
export function IframeAppDocument({ children, description }: IframeAppDocumentProps) {
  return (
    <html lang="en" data-theme="dark" suppressHydrationWarning>
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
        <script dangerouslySetInnerHTML={{ __html: buildInlineThemeBootstrap() }} />
      </head>
      <body>
        {children}
        <ScrollRestoration />
        <Scripts />
      </body>
    </html>
  );
}
