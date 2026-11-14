import { llms, loader } from 'fumadocs-core/source';
import { docsRoute, linkBase } from './shared';
import { defineDocs } from 'fumadocs-mdx/macro';

const docs = defineDocs({
  docs: {
    postprocess: {
      includeProcessedMarkdown: true,
    },
  },
});

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: docsRoute,
  source: docs.toFumadocsSource(),
});

export const docsLlms = llms(source, {
  renderPage: async (page) => `# ${page.data.title} (${linkBase}${page.url})

${(await page.data.getText('processed')).replace(/\]\(\/(?!\/)/g, `](${linkBase}/`)}`,
});
