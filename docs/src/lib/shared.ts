import { defineSite } from '@weebo/operator-docs/site';

// What makes this site weebo-authentik's; everything else comes from
// @weebo/operator-docs.
export const site = defineSite({
  name: 'weebo-authentik',
  description:
    'Manage Authentik groups, users, applications and access policies as Kubernetes resources.',
  git: {
    user: 'batleforc',
    repo: 'weebo-authentik',
    branch: 'main',
    contentDir: 'docs/content/docs',
  },
});
