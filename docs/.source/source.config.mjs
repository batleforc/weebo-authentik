// source.config.ts
import { defineConfig, defineDocs } from "fumadocs-mdx/config";
import { metaSchema, pageSchema } from "fumadocs-core/source/schema";
var docs = defineDocs({
  dir: "content/docs",
  docs: {
    schema: pageSchema,
    postprocess: {
      includeProcessedMarkdown: true
    }
  },
  meta: {
    schema: metaSchema
  }
});
var source_config_default = defineConfig({
  mdxOptions: {
    // The default github-light/dark themes miss WCAG AA on the BatleHub
    // grounds; the high-contrast pair is what batlehub's docs measured clean.
    rehypeCodeOptions: {
      themes: {
        light: "github-light-high-contrast",
        dark: "github-dark-high-contrast"
      }
    }
  }
});
export {
  source_config_default as default,
  docs
};
