export const appName = 'weebo-authentik';

// GitHub Pages serves this project repo under a sub-path, which the static
// export bakes in as `basePath`/`assetPrefix`. Next rewrites framework URLs
// (links, assets) for us, but NOT runtime `fetch()` of files in `public/`, so
// client code that fetches a static asset must prefix this itself. Kept in
// lockstep with next.config.mjs via the same NEXT_PUBLIC_BASE_PATH env var;
// empty in `npm run dev`, so the site still serves at `/` locally.
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';

export const docsRoute = '/docs';
export const docsImageRoute = '/og/docs';
export const docsContentRoute = '/llms.mdx/docs';

export const gitConfig = {
  user: 'batleforc',
  repo: 'weebo-authentik',
  branch: 'main',
};
