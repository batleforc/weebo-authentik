import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

// GitHub Pages serves this project repo under a sub-path
// (https://batleforc.github.io/weebo-authentik/), so the static export bakes
// that in as basePath/assetPrefix. It is env-gated — `npm run dev` leaves
// NEXT_PUBLIC_BASE_PATH unset and the site serves at `/` locally, while CI
// builds with NEXT_PUBLIC_BASE_PATH=/weebo-authentik. Keep this in lockstep
// with the `basePath` constant in src/lib/shared.ts (runtime fetches of
// public/ assets, which Next does not rewrite, read it from the same env var).
const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';

/** @type {import('next').NextConfig} */
const config = {
  reactStrictMode: true,
  // Static HTML export — GitHub Pages has no Node server. Search, the OG
  // images and the llms.* routes are all built to static files (see each
  // route's force-static / generateStaticParams).
  output: 'export',
  basePath,
  assetPrefix: basePath,
  // GitHub Pages resolves `/foo/` → `/foo/index.html`; trailing slashes keep
  // every route addressable as a directory.
  trailingSlash: true,
  // next/image optimisation needs a server; static export can't run it.
  images: { unoptimized: true },
  allowedDevOrigins: ["*.cde.batleforc.fr"]
};

export default withMDX(config);
