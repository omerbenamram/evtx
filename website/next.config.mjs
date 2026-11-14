import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

const basePath = process.env.DOCS_BASE_PATH ?? '';

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  basePath: basePath || undefined,
  // Inlined into client code too, such as the search dialog's index URL.
  env: { DOCS_BASE_PATH: basePath },
  reactStrictMode: true,
};

export default withMDX(config);
