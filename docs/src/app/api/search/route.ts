import { searchRoute } from '@weebo/operator-docs/routes';
import { source } from '@/lib/source';

// `output: 'export'` can't serve a dynamic handler, so the search index is
// built to a static file at /api/search and queried client-side (the dialog
// is configured with `type: 'static'` by DocsRootProvider).
export const revalidate = false;

export const GET = searchRoute(source);
