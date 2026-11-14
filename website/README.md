# Docs site

The docs for evtx, mft, pyevtx-rs and pymft-rs, built with [Fumadocs](https://fumadocs.dev) as a static site.

```bash
npm ci
npm run dev     # http://localhost:3000
npm run build   # static site in out/
```

Pages are MDX files in `content/docs/`; each folder's `meta.json` sets the sidebar order.

## Writing a page

The reader is an analyst with a file and a question. Write the page a colleague would want:

- Start with the situation and a command, then its output. Explain after.
- Run every command and paste what it printed. When the output is too long, cut it with a filter in the command (`jq`, `cut`, `head`), not by hand, so that the reader gets the same thing. The event log examples use `samples/` of this repository. The NTFS examples use the Narcos-1 image from Digital Corpora (`narcos-ccleaner.MFT` and the files extracted from the same volume).
- Give numbers you measured, with the size of the input, in place of words such as "fast".
- Say what goes wrong and what a result does not prove: dirty logs, reused records, a slack entry that is not a deleted file.
- A feature that is merged but not released gets a `<Callout title="Not in a release yet" type="warn">` that names the version it ships with, or says "the next release" when the number is not decided. Remove the note in the release pull request.

The build also writes, for agents:

- `/llms.txt`: an index of every page, linking to its markdown.
- `/llms-full.txt`: every page in one file.
- `/llms.mdx/docs/<page>/content.md`: one page as markdown.

`DOCS_SITE_URL` (the origin, such as `https://omerbenamram.github.io`) and `DOCS_BASE_PATH` (such as `/docs`) set where the site is served from, so links in `llms.txt` are absolute.
