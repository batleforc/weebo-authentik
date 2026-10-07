import Link from 'next/link';

// A docs index is Read, not Persuade — the visitor is evaluating, so the hero
// states what the product is and gets out of the way (RFC 0005 §4.1). No glow,
// no grid, no glitch, no gradient-clip: hairline rules, square corners, the
// crimson accent used once. The title is Silkscreen at an integer-em step
// (40px below 640px, 72px above — higher steps overflow "weebo-authentik").
const steps = [
  {
    n: '01',
    title: 'Install via Helm',
    body: 'Pull the chart from GHCR and bring up the operator and its admission webhook.',
    href: '/docs/guides/install',
  },
  {
    n: '02',
    title: 'Connect an AuthentikInstance',
    body: 'Point the operator at your Authentik and hand it an API token.',
    href: '/docs/guides/connect-instance',
  },
  {
    n: '03',
    title: 'Declare your first application',
    body: 'Apply a CRD and watch it reconcile into a real Authentik application.',
    href: '/docs/guides/first-application',
  },
];

export default function HomePage() {
  return (
    <div className="flex flex-1 flex-col items-center px-5 py-16 sm:py-24">
      <div className="flex w-full max-w-3xl flex-col gap-10">
        <header className="flex flex-col gap-5">
          <span className="font-mono text-xs font-semibold uppercase tracking-[0.16em] text-fd-muted-foreground">
            Kubernetes Operator
          </span>

          <h1
            className="font-(family-name:--face-display) font-bold tracking-[0.02em] text-fd-foreground"
            style={{ fontSize: 'var(--t-display)', lineHeight: 0.92 }}
          >
            weebo-authentik
          </h1>

          <p className="max-w-[68ch] text-base leading-[1.7] text-fd-muted-foreground">
            Manage Authentik groups, users, applications and access policies as
            native Kubernetes custom resources, with a namespace-scoped
            allow-list enforced via an admission webhook.
          </p>

          <div className="flex flex-wrap gap-3 pt-1">
            <Link
              href="/docs"
              className="inline-flex items-center border border-fd-primary bg-fd-primary px-5 py-2.5 font-mono text-sm font-semibold uppercase tracking-wide text-fd-primary-foreground transition-colors hover:bg-fd-primary/90"
            >
              Read the docs
            </Link>
            <Link
              href="/docs/guides/build-a-manifest"
              className="inline-flex items-center border border-fd-border px-5 py-2.5 font-mono text-sm font-semibold uppercase tracking-wide text-fd-muted-foreground transition-colors hover:border-fd-primary hover:text-fd-primary"
            >
              Build a manifest
            </Link>
          </div>
        </header>

        <section className="flex flex-col">
          <h2 className="border-b border-fd-border pb-2 font-mono text-xs font-semibold uppercase tracking-[0.14em] text-fd-muted-foreground">
            Quick start
          </h2>
          <ol className="flex flex-col">
            {steps.map((step) => (
              <li key={step.n} className="border-b border-dashed border-fd-border/60">
                <Link
                  href={step.href}
                  className="group flex items-baseline gap-4 py-4 transition-colors hover:bg-fd-muted/50"
                >
                  <span className="font-mono text-sm tabular-nums text-fd-primary">
                    {step.n}
                  </span>
                  <span className="flex flex-col gap-1">
                    <span className="font-mono text-sm font-semibold uppercase tracking-wide text-fd-foreground group-hover:underline">
                      {step.title}
                    </span>
                    <span className="text-sm leading-[1.7] text-fd-muted-foreground">
                      {step.body}
                    </span>
                  </span>
                </Link>
              </li>
            ))}
          </ol>
        </section>
      </div>
    </div>
  );
}
