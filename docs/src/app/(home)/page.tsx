import Link from 'next/link';
import { gitConfig } from '@/lib/shared';

// The proof sheet: one form at poster scale captioned by its facts, then
// ruled regions — no card, no glow, no gradient (DESIGN.md, Overview).

const repoUrl = `https://github.com/${gitConfig.user}/${gitConfig.repo}`;

const features = [
  {
    title: 'Declare Authentik',
    body: 'Groups, users, applications, brands, flows, scope mappings and outposts are Kubernetes resources. Apply YAML, and the operator keeps Authentik in line with it.',
    href: '/docs/guides/first-application',
    link: 'First application',
  },
  {
    title: 'Hand out credentials',
    body: 'An application’s OAuth2 client ID and secret are written straight into a Secret (or Vault) in its namespace — never pasted by hand.',
    href: '/docs/guides/connect-instance',
    link: 'Connect an instance',
  },
  {
    title: 'Fence namespaces',
    body: 'An allow-list says which namespaces may declare applications. It is enforced by an admission webhook, before anything reaches Authentik.',
    href: '/docs/guides/allow-list',
    link: 'Allow-list guide',
  },
];

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col px-4 sm:px-6">
      <section className="mx-auto flex w-full max-w-[1100px] flex-col gap-6 pt-10 pb-10 sm:pt-16">
        <span className="sheet-label">Kubernetes operator · Authentik</span>
        <h1 className="sheet-display">weebo-authentik</h1>
        <p className="sheet-caption">
          Manage Authentik groups, users, applications and access policies as native Kubernetes
          custom resources, with GitOps-friendly status and a namespace-scoped allow-list enforced
          by an admission webhook.
        </p>
        <div className="flex flex-wrap items-center gap-3">
          <Link href="/docs/guides/install" className="action">
            Get started
          </Link>
          <Link href="/docs" className="ctl">
            Read the docs
          </Link>
          <Link href="/docs/guides/build-a-manifest" className="ctl">
            Build a manifest
          </Link>
          <a href={repoUrl} className="ctl" rel="noreferrer">
            View on GitHub
          </a>
        </div>
      </section>

      <section className="sheet-region mx-auto w-full max-w-[1100px]">
        <div className="grid gap-6 md:grid-cols-2">
          <div className="flex min-w-0 flex-col gap-3">
            <span className="sheet-label">1 · Install the chart</span>
            <pre className="sheet-pre">
              <code>
                helm install weebo-authentik \{'\n'}
                {'  '}oci://ghcr.io/batleforc/charts/weebo-authentik \{'\n'}
                {'  '}--namespace weebo-authentik --create-namespace \{'\n'}
                {'  '}--set certManager.issuerRef.name=&lt;issuer&gt;
              </code>
            </pre>
          </div>
          <div className="flex min-w-0 flex-col gap-3">
            <span className="sheet-label">2 · Declare an application</span>
            <pre className="sheet-pre">
              <code>
                <span className="k">apiVersion:</span> <span className="v">authentik.weebo.io/v1alpha1</span>
                {'\n'}
                <span className="k">kind:</span> <span className="v">AuthentikApplication</span>
                {'\n'}
                <span className="k">metadata:</span>
                {'\n'}
                {'  '}
                <span className="k">name:</span> <span className="v">harbor</span>
                {'\n'}
                {'  '}
                <span className="k">namespace:</span> <span className="v">team-a</span>
                {'\n'}
                <span className="k">spec:</span>
                {'\n'}
                {'  '}
                <span className="k">instanceRef:</span> <span className="v">main</span>
                {'\n'}
                {'  '}
                <span className="k">name:</span> <span className="v">Harbor</span>
                {'\n'}
                {'  '}
                <span className="k">slug:</span> <span className="v">harbor</span>
                {'\n'}
                {'  '}
                <span className="k">provider:</span> <span className="v">{'{ kind: oauth2, … }'}</span>
              </code>
            </pre>
          </div>
        </div>
      </section>

      <section className="sheet-region mx-auto grid w-full max-w-[1100px] md:grid-cols-3">
        {features.map((f) => (
          <div key={f.title} className="sheet-feature flex flex-col gap-3 py-6 md:px-6 md:py-0 md:first:ps-0">
            <h2>{f.title}</h2>
            <p className="text-[13px] leading-[1.6] text-fd-muted-foreground">{f.body}</p>
            <Link href={f.href}>{f.link}</Link>
          </div>
        ))}
      </section>
    </main>
  );
}
