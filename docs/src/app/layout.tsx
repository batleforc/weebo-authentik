import type { Metadata } from 'next';
import { RootProvider } from 'fumadocs-ui/provider/next';
import './global.css';
import { JetBrains_Mono, Silkscreen } from 'next/font/google';
import { appName, basePath } from '@/lib/shared';

// Absolute base for OG/social image + canonical URLs. Without it Next resolves
// them against localhost:3000, which ships broken meta tags on the static
// export. Includes the Pages sub-path so relative image URLs land under it;
// override via NEXT_PUBLIC_SITE_URL for a custom domain.
export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL ?? 'https://batleforc.github.io/weebo-authentik',
  ),
  title: { default: appName, template: `%s · ${appName}` },
  description:
    'Manage Authentik groups, users, applications and access policies as Kubernetes resources.',
};

// The BatleHub pairing: a square-pixel display face for identity and scale,
// a mono text face for every fact. next/font self-hosts both at build time.
const silkscreen = Silkscreen({
  subsets: ['latin'],
  weight: ['400', '700'],
  variable: '--font-silkscreen',
  display: 'swap',
});

const jetbrainsMono = JetBrains_Mono({
  subsets: ['latin'],
  weight: ['400', '500', '600', '700', '800'],
  variable: '--font-jetbrains-mono',
  display: 'swap',
});

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html
      lang="en"
      className={`${silkscreen.variable} ${jetbrainsMono.variable} font-sans`}
      suppressHydrationWarning
    >
      <body className="flex flex-col min-h-screen">
        <RootProvider
          // `data-theme` is what the BatleHub tokens key off; `class` is what
          // Fumadocs' own `dark:` utilities key off. The stored value is the
          // preference (system|light|dark), resolved before first paint.
          theme={{ attribute: ['class', 'data-theme'], defaultTheme: 'system', enableSystem: true }}
          search={{ options: { type: 'static', api: `${basePath}/api/search` } }}
        >
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
