import type { Node } from 'fumadocs-core/page-tree';
import { source } from '@/lib/source';
import { appDescription, appName, getPageMarkdownUrl, siteUrl } from '@/lib/shared';

export const revalidate = false;

const sectionTitles: Record<string, string> = {
  '': 'Start here',
  cli: 'Command line',
  python: 'Python',
  rust: 'Rust',
  guides: 'Guides: when to use what',
};

// Page URLs in sidebar order.
function pageUrls(nodes: Node[]): string[] {
  return nodes.flatMap((node) => {
    if (node.type === 'page') return [node.url];
    if (node.type === 'folder') return [...(node.index ? [node.index.url] : []), ...pageUrls(node.children)];
    return [];
  });
}

// https://llmstxt.org: a title, a summary, then links to each page's markdown, grouped by the
// folder the page is in.
export async function GET() {
  const lines = [
    `# ${appName}`,
    '',
    `> ${appDescription}`,
    '',
    `Every link below is the page as plain markdown. All pages in one file: ${siteUrl}/llms-full.txt`,
  ];
  const pages = new Map(source.getPages().map((page) => [page.url, page]));
  let section: string | undefined;
  for (const url of pageUrls(source.getPageTree().children)) {
    const page = pages.get(url);
    if (!page) continue;
    const folder = page.slugs.length > 1 ? page.slugs[0] : '';
    if (folder !== section) {
      section = folder;
      lines.push('', `## ${sectionTitles[folder] ?? folder}`, '');
    }
    const markdownUrl = `${siteUrl}${getPageMarkdownUrl(page).url}`;
    lines.push(`- [${page.data.title}](${markdownUrl}): ${page.data.description ?? ''}`);
  }
  return new Response(`${lines.join('\n')}\n`);
}
