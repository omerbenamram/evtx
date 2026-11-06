import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  output: 'export',
  basePath: process.env.DOCS_BASE_PATH || undefined,
  reactStrictMode: true,
};

export default withMDX(config);
