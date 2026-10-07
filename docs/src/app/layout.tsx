import type { Metadata } from 'next';
import { RootProvider } from 'fumadocs-ui/provider/next';
import './global.css';
import { JetBrains_Mono, Silkscreen } from 'next/font/google';
import { basePath } from '@/lib/shared';

// Absolute base for OG/social image + canonical URLs. Without it Next resolves
// them against localhost:3000, which ships broken meta tags on the static
// export. Includes the Pages sub-path so relative image URLs land under it;
// override via NEXT_PUBLIC_SITE_URL for a custom domain.
export const metadata: Metadata = {
  metadataBase: new URL(
    process.env.NEXT_PUBLIC_SITE_URL ?? 'https://batleforc.github.io/weebo-authentik',
  ),
};

// Silkscreen is the display face — headings and pixel-scale labels only, never
// body copy (it is drawn on an 8px em). JetBrains Mono carries every fact.
// next/font self-hosts both at build time, so no request leaves for a font —
// the rule DESIGN.md's Typography section makes law.
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
      className={`${silkscreen.variable} ${jetbrainsMono.variable}`}
      suppressHydrationWarning
    >
      <body className="flex flex-col min-h-screen">
        {/* Both renditions are first-class — paper is the home ground, near-black
            the adaptation. The preference (system|light|dark) is stored, never
            the resolved value, and resolved before first paint. */}
        <RootProvider
          theme={{ defaultTheme: 'system', enableSystem: true }}
          search={{ options: { type: 'static', api: `${basePath}/api/search` } }}
        >
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
