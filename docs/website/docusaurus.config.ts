import {themes as prismThemes} from 'prism-react-renderer';
import type {Config} from '@docusaurus/types';
import type * as Preset from '@docusaurus/preset-classic';

const config: Config = {
  title: 'ctx',
  tagline: 'AI-ready context from your codebase',
  favicon: 'img/favicon.ico',

  future: {
    v4: true,
  },

  url: 'https://saldestechnology.github.io',
  baseUrl: '/ctx/',

  organizationName: 'saldestechnology',
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
          editUrl: 'https://github.com/saldestechnology/ctx/tree/main/docs/website/',
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
          href: 'https://github.com/saldestechnology/ctx',
          label: 'GitHub',
          position: 'right',
        },
      ],
    },
    footer: {
      style: 'dark',
      links: [
        {
          title: 'Documentation',
          items: [
            {
              label: 'Getting Started',
              to: '/docs/getting-started',
            },
            {
              label: 'Context Generation',
              to: '/docs/context-generation',
            },
            {
              label: 'Code Intelligence',
              to: '/docs/code-intelligence',
            },
          ],
        },
        {
          title: 'More',
          items: [
            {
              label: 'GitHub',
              href: 'https://github.com/saldestechnology/ctx',
            },
          ],
        },
      ],
      copyright: `Copyright ${new Date().getFullYear()} ctx. Built with Docusaurus.`,
    },
    prism: {
      theme: prismThemes.github,
      darkTheme: prismThemes.dracula,
      additionalLanguages: ['bash', 'rust', 'typescript', 'python', 'go', 'solidity', 'sql', 'toml', 'yaml'],
    },
  } satisfies Preset.ThemeConfig,
};

export default config;
