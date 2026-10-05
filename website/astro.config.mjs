import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { analyticsHead } from './src/analytics.mjs';
export default defineConfig({
  site: 'https://graphfusion.github.io',
  trailingSlash: 'always',
  integrations: [starlight({
    head: analyticsHead,
    title: 'GraphFusion',
    description: 'An embedded graph database in Rust. Query property graphs with GQL, execute with Apache DataFusion, and store data in Arrow and Parquet.',
    favicon: '/favicon.svg',
    logo: { src: './src/assets/graphfusion.svg', alt: '', replacesTitle: false },
    expressiveCode: { shiki: { langAlias: { gql: 'cypher' } } },
    social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/GraphFusion/GraphFusion' }],
    components: { SiteTitle: './src/components/HeaderBrand.astro' },
    customCss: ['./src/styles/custom.css'],
    sidebar: [
      { label: 'Start here', items: [{ autogenerate: { directory: 'start' } }] },
      { label: 'Query language', items: [{ autogenerate: { directory: 'query' } }] },
      { label: 'Graph patterns & paths', items: [{ autogenerate: { directory: 'patterns' } }] },
      { label: 'Functions & expressions', items: [{ autogenerate: { directory: 'expressions' } }], collapsed: true },
      { label: 'Write data', items: [{ autogenerate: { directory: 'mutations' } }], collapsed: true },
      { label: 'Catalog & sessions', items: [{ autogenerate: { directory: 'catalog' } }], collapsed: true },
      { label: 'Transactions & storage', items: [{ autogenerate: { directory: 'storage' } }], collapsed: true },
      { label: 'Rust API', items: [{ autogenerate: { directory: 'rust' } }], collapsed: true },
      { label: 'Development', items: [{ autogenerate: { directory: 'development' } }], collapsed: true },
    ],
    tableOfContents: { minHeadingLevel: 2, maxHeadingLevel: 3 },
  })],
});
