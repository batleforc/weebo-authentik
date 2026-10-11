import { withOperatorDocs } from '@weebo/operator-docs/next';

// GitHub Pages serves this project repo under a sub-path
// (https://batleforc.github.io/weebo-authentik/), so the static export bakes
// that in as basePath. It is env-gated — `npm run dev` leaves
// NEXT_PUBLIC_BASE_PATH unset and the site serves at `/` locally, while CI
// builds with NEXT_PUBLIC_BASE_PATH=/weebo-authentik. withOperatorDocs passes
// it on to client code (runtime fetches of public/ assets, which Next does not
// rewrite).
export default withOperatorDocs({
  basePath: process.env.NEXT_PUBLIC_BASE_PATH ?? '',
  allowedDevOrigins: ['*.cde.batleforc.fr'],
});
