# docs

This is a Next.js application generated with
[Create Fumadocs](https://github.com/fuma-nama/fumadocs).

Run development server:

```bash
npm run dev
# or
pnpm dev
# or
yarn dev
```

Open http://localhost:3000 with your browser to see the result.

## Explore

In the project, you can see:

- `lib/source.ts`: Code for content source adapter, [`loader()`](https://fumadocs.dev/docs/headless/source-api) provides the interface to access your content.
- `lib/shared.ts`: the site's name, pitch and repository; `lib/crd-choices.ts`: the manifest builder's guided choices.
- `crd-docs.config.mjs`: input of `weebo-crd-docs` (`npm run gen:crds`, run by `task recu`), which generates `content/docs/crds/` and `public/crd-schemas/` from `deploy/crd/`, and `content/docs/reference/` (reason codes, Events, metrics & health, resource budget) from `crates/` and `perf/weebo-authentik.json` (the last `task perf` report, copied from `target/perf/`).

Everything else the weebo operators' sites share (CRD page generator,
manifest builder, theme, layouts, docs page, search/llms/OG routes) comes
from [`@weebo/operator-docs`](https://github.com/batleforc/weebo-operator-kit/tree/main/packages/operator-docs),
the kit release's tarball in `package.json`.

| Route                     | Description                                            |
| ------------------------- | ------------------------------------------------------ |
| `app/(home)`              | The route group for your landing page and other pages. |
| `app/docs`                | The documentation layout and pages.                    |
| `app/api/search/route.ts` | The Route Handler for search.                          |

### Fumadocs MDX

A `source.config.ts` config file has been included, you can customise different options like frontmatter schema.

Read the [Introduction](https://fumadocs.dev/docs/mdx) for further details.

## Learn More

To learn more about Next.js and Fumadocs, take a look at the following
resources:

- [Next.js Documentation](https://nextjs.org/docs) - learn about Next.js
  features and API.
- [Learn Next.js](https://nextjs.org/learn) - an interactive Next.js tutorial.
- [Fumadocs](https://fumadocs.dev) - learn about Fumadocs
