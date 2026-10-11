import { docsMdxComponents } from '@weebo/operator-docs/site';
import type { MDXComponents } from 'mdx/types';
import { CHOICES } from '@/lib/crd-choices';

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...docsMdxComponents(CHOICES),
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
