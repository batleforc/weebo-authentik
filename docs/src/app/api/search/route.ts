import { source } from '@/lib/source';
import { createFromSource } from 'fumadocs-core/search/server';

// `output: 'export'` can't serve a dynamic handler, so the search index is
// built to a static file at /api/search and queried client-side (the dialog
// is configured with `type: 'static'` in layout.tsx). `staticGET` is the
// build-time export of the same index `GET` would have served dynamically.
export const revalidate = false;

export const { staticGET: GET } = createFromSource(source, {
  // https://docs.orama.com/docs/orama-js/supported-languages
  language: 'english',
});
