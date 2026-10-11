import { docs } from 'collections/server';
import { loader } from 'fumadocs-core/source';
import { docsLoaderOptions } from '@weebo/operator-docs/site';

export const source = loader({
  source: docs.toFumadocsSource(),
  ...docsLoaderOptions,
});
