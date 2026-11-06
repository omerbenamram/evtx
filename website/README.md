# Docs site

The docs for evtx, mft, pyevtx-rs and pymft-rs, built with [Fumadocs](https://fumadocs.dev) as a static site.

```bash
npm ci
npm run dev     # http://localhost:3000
npm run build   # static site in out/
```

Pages are MDX files in `content/docs/`; each folder's `meta.json` sets the sidebar order. Keep pages example-first and short: a command, its real output, then a table of when to use what.

The build also writes, for agents:

- `/llms.txt`: an index of every page, linking to its markdown.
- `/llms-full.txt`: every page in one file.
- `/llms.mdx/docs/<page>/content.md`: one page as markdown.

`DOCS_SITE_URL` (the origin, such as `https://omerbenamram.github.io`) and `DOCS_BASE_PATH` (such as `/docs`) set where the site is served from, so links in `llms.txt` are absolute.
