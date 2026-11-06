import type { Node } from 'fumadocs-core/page-tree';
import { source } from '@/lib/source';
import { appDescription, appName, getPageMarkdownUrl, siteUrl } from '@/lib/shared';

export const revalidate = false;

function pageLink(node: Node): string[] {
  if (node.type !== 'page') return [];
  const page = source.getNodePage(node);
  if (!page) return [];
  const url = `${siteUrl}${getPageMarkdownUrl(page).url}`;
  return [`- [${page.data.title}](${url}): ${page.data.description ?? ''}`];
}

// https://llmstxt.org: a title, a summary, then links to each page's markdown in sidebar order,
// under the sidebar's folder titles (meta.json).
export async function GET() {
  const tree = source.getPageTree().children;
  const lines = [
    `# ${appName}`,
    '',
    `> ${appDescription}`,
    '',
    `Every link below is the page as plain markdown. All pages in one file: ${siteUrl}/llms-full.txt`,
    '',
    '## Start here',
    '',
    ...tree.flatMap(pageLink),
  ];
  for (const node of tree) {
    if (node.type !== 'folder') continue;
    lines.push('', `## ${node.name}`, '', ...[node.index, ...node.children].flatMap((n) => (n ? pageLink(n) : [])));
  }
  return new Response(`${lines.join('\n')}\n`);
}
