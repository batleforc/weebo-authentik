import { OperatorHome } from '@weebo/operator-docs/site';
import { site } from '@/lib/shared';

export default function HomePage() {
  return (
    <OperatorHome
      site={site}
      label="Kubernetes operator · Authentik"
      caption="Manage Authentik groups, users, applications and access policies as native Kubernetes custom resources, with GitOps-friendly status and a namespace-scoped allow-list enforced by an admission webhook."
      actions={[
        { href: '/docs/guides/install', label: 'Get started', primary: true },
        { href: '/docs', label: 'Read the docs' },
        { href: '/docs/ide', label: 'Access the CRD IDE' },
        { href: '/docs/guides/build-a-manifest', label: 'Build a manifest' },
      ]}
      install={`helm install weebo-authentik \\
  oci://ghcr.io/batleforc/charts/weebo-authentik \\
  --namespace weebo-authentik --create-namespace \\
  --set certManager.issuerRef.name=<issuer>`}
      example={{
        title: 'Declare an application',
        yaml: `apiVersion: authentik.weebo.io/v1alpha1
kind: AuthentikApplication
metadata:
  name: harbor
  namespace: team-a
spec:
  instanceRef: main
  name: Harbor
  slug: harbor
  provider: { kind: oauth2, … }`,
      }}
      features={[
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
      ]}
    />
  );
}
