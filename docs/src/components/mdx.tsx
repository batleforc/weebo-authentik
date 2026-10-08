import defaultMdxComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import { CrdForm } from './crd-form/CrdForm';
import { CrdBuilder } from './crd-form/CrdBuilder';

export function getMDXComponents(components?: MDXComponents) {
  return {
    ...defaultMdxComponents,
    CrdForm,
    CrdBuilder,
    ...components,
  } satisfies MDXComponents;
}

export const useMDXComponents = getMDXComponents;

declare global {
  type MDXProvidedComponents = ReturnType<typeof getMDXComponents>;
}
