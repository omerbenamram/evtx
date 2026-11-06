import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { appName } from './shared';

const repos = ['evtx', 'mft', 'pyevtx-rs', 'pymft-rs'];

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: appName,
    },
    links: repos.map((repo) => ({
      text: repo,
      url: `https://github.com/omerbenamram/${repo}`,
      secondary: true,
    })),
  };
}
