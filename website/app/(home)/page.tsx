import Link from 'next/link';
import { basePath } from '@/lib/shared';

const artifacts = [
  { name: '.evtx event logs', tool: 'evtx_dump', href: '/docs/cli/evtx-dump' },
  { name: '$MFT', tool: 'mft_dump', href: '/docs/cli/mft-dump' },
  { name: '$UsnJrnl:$J, $I30, $LogFile, $Boot, $Secure:$SDS', tool: 'mft_dump', href: '/docs/cli/ntfs-files' },
  { name: 'Disk images, unallocated space', tool: 'evtx_dump --carve', href: '/docs/guides/deleted-events' },
];

const paths = [
  {
    title: 'Command line',
    text: 'One binary per format. Point it at a file, get JSON, CSV or XML.',
    code: 'evtx_dump -o jsonl Security.evtx | jq .',
    href: '/docs/cli/evtx-dump',
  },
  {
    title: 'Python',
    text: 'pip install, iterate records, feed your pipeline or SIEM.',
    code: 'pip install evtx mft',
    href: '/docs/python/evtx',
  },
  {
    title: 'Rust',
    text: 'The same parsers as crates. Safe Rust, multithreaded.',
    code: 'cargo add evtx mft',
    href: '/docs/rust/evtx',
  },
];

export default function HomePage() {
  return (
    <main className="mx-auto flex w-full max-w-5xl flex-1 flex-col gap-14 px-4 py-16">
      <section className="flex flex-col gap-4">
        <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">
          Parse Windows forensic artifacts. Fast.
        </h1>
        <p className="max-w-2xl text-lg text-fd-muted-foreground">
          Event logs and NTFS metadata to JSON, CSV or XML, from the command line, Python or Rust.
          Open source, safe Rust, built for triage at scale.
        </p>
        <div className="flex flex-wrap gap-3">
          <Link
            href="/docs"
            className="rounded-lg bg-fd-primary px-4 py-2 text-sm font-medium text-fd-primary-foreground"
          >
            Start here
          </Link>
          <Link href="/docs/install" className="rounded-lg border px-4 py-2 text-sm font-medium">
            Install
          </Link>
        </div>
      </section>

      <section className="grid gap-4 md:grid-cols-3">
        {paths.map((path) => (
          <Link
            key={path.title}
            href={path.href}
            className="flex flex-col gap-3 rounded-xl border bg-fd-card p-5 transition-colors hover:bg-fd-accent"
          >
            <h2 className="font-semibold">{path.title}</h2>
            <p className="text-sm text-fd-muted-foreground">{path.text}</p>
            <code className="mt-auto rounded-md bg-fd-muted px-3 py-2 font-mono text-xs">{path.code}</code>
          </Link>
        ))}
      </section>

      <section className="flex flex-col gap-4">
        <h2 className="text-xl font-semibold">What do you have?</h2>
        <div className="divide-y rounded-xl border">
          {artifacts.map((artifact) => (
            <Link
              key={artifact.name}
              href={artifact.href}
              className="flex flex-col justify-between gap-1 px-5 py-3 transition-colors hover:bg-fd-accent sm:flex-row"
            >
              <span>{artifact.name}</span>
              <code className="font-mono text-sm text-fd-muted-foreground">{artifact.tool}</code>
            </Link>
          ))}
        </div>
      </section>

      <p className="text-sm text-fd-muted-foreground">
        Using an AI agent? Point it at <a href={`${basePath}/llms.txt`} className="underline">
          llms.txt
        </a>: every
        page is also plain markdown.
      </p>
    </main>
  );
}
