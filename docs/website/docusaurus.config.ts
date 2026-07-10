import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

// Code highlighting is handled by Shiki via the swizzled @theme/CodeBlock
// component (src/theme/CodeBlock). Prism is left unconfigured on purpose.

const config: Config = {
  title: 'ctx',
  tagline: 'A queryable world model of your codebase',
  favicon: 'img/favicon.svg',

  future: {
    v4: true,
  },

  url: 'https://docs.agentis.tools',
  baseUrl: '/',

  organizationName: 'agentis-tools',
  projectName: 'ctx',

  onBrokenLinks: 'throw',

  markdown: {
    hooks: {
      onBrokenMarkdownLinks: 'warn',
    },
  },

  i18n: {
    defaultLocale: 'en',
    locales: ['en'],
  },

  presets: [
    [
      'classic',
      {
        docs: {
          sidebarPath: './sidebars.ts',
          editUrl: 'https://github.com/agentis-tools/ctx/tree/main/docs/website/',
        },
        blog: false,
        theme: {
          customCss: './src/css/custom.css',
        },
      } satisfies Preset.Options,
    ],
  ],

  themeConfig: {
    colorMode: {
      defaultMode: 'dark',
      respectPrefersColorScheme: true,
    },
    navbar: {
      title: 'ctx',
      logo: {
        alt: 'ctx Logo',
        src: 'img/logo.svg',
      },
      items: [
        {
          type: 'docSidebar',
          sidebarId: 'docsSidebar',
          position: 'left',
          label: 'Documentation',
        },
        {
          href: 'https://github.com/agentis-tools/ctx',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Docs',
          items: [
            {label: 'Why ctx?', to: '/docs/why-ctx'},
            {label: 'Getting Started', to: '/docs/getting-started'},
            {label: 'Using ctx with agents', to: '/docs/guides/using-ctx-with-agents'},
            {label: 'Comparison', to: '/docs/comparison'},
          ],
        },
        {
          title: 'More',
          items: [
            {label: 'GitHub', href: 'https://github.com/agentis-tools/ctx'},
            {label: 'llms.txt', href: 'pathname:///llms.txt'},
          ],
        },
      ],
      copyright: `Copyright ${new Date().getFullYear()} ctx. Built with Docusaurus.`,
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
