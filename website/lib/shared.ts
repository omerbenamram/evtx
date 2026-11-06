import { createGetUrl } from 'fumadocs-core/source';

export const appName = 'DFIR Parsers';
export const appDescription =
  'Fast, safe parsers for Windows forensic artifacts: event logs (.evtx), $MFT, $UsnJrnl, $LogFile and more. CLI tools, Python packages and Rust crates.';

// Where the site is served from: `DOCS_BASE_PATH` is the path prefix (such as `/docs` on
// GitHub Pages; next.config.mjs inlines it into client code too), and `DOCS_SITE_URL` the
// origin, so that llms.txt can give absolute links.
export const basePath = process.env.DOCS_BASE_PATH ?? '';
export const siteUrl = `${process.env.DOCS_SITE_URL || 'http://localhost:3000'}${basePath}`;

export const docsRoute = '/docs';
export const editUrlBase = 'https://github.com/omerbenamram/evtx/blob/master/website/content/docs';

// `/llms.mdx/docs/<slugs>/content.md` and `/og/docs/<slugs>/image.png`.
function pageFileUrl(route: string, file: string) {
  const getUrl = createGetUrl(route);
  return (page: { slugs: string[]; locale?: string }) => {
    const segments = [...page.slugs, file];
    return { segments, url: getUrl(segments, page.locale) };
  };
}

export const getPageMarkdownUrl = pageFileUrl('/llms.mdx/docs', 'content.md');
export const getPageImageUrl = pageFileUrl('/og/docs', 'image.png');
