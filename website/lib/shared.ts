import { createGetUrl } from 'fumadocs-core/source';

export const appName = 'DFIR Parsers';
export const appDescription =
  'Fast, safe parsers for Windows forensic artifacts: event logs (.evtx), $MFT, $UsnJrnl, $LogFile and more. CLI tools, Python packages and Rust crates.';

// Where the site is served from. `DOCS_BASE_PATH` is the path prefix (such as `/docs` on
// GitHub Pages), and `DOCS_SITE_URL` the origin, so that llms.txt can give absolute links.
export const basePath = process.env.DOCS_BASE_PATH ?? '';
export const siteUrl = `${process.env.DOCS_SITE_URL ?? 'http://localhost:3000'}${basePath}`;

export const docsRoute = '/docs';
export const docsImageRoute = '/og/docs';
export const docsContentRoute = '/llms.mdx/docs';

export const gitConfig = {
  user: 'omerbenamram',
  repo: 'evtx',
  branch: 'master',
  // The site's folder in the repository.
  dir: 'website',
};

const getContentUrl = createGetUrl(docsContentRoute);

export function getPageMarkdownUrl(page: { slugs: string[]; locale?: string }) {
  const segments = [...page.slugs, 'content.md'];

  return { segments, url: getContentUrl(segments, page.locale) };
}

const getImageUrl = createGetUrl(docsImageRoute);

export function getPageImageUrl(page: { slugs: string[]; locale?: string }) {
  const segments = [...page.slugs, 'image.png'];

  return { segments, url: getImageUrl(segments, page.locale) };
}
