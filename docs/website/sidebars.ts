import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
    'why-ctx',
    'comparison',
    'getting-started',
    {
      type: 'category',
      label: 'Guides',
      items: [
        'guides/using-ctx-with-agents',
      ],
    },
    'context-generation',
    'code-intelligence',
    {
      type: 'category',
      label: 'Governance',
      items: [
        'governance/overview',
        'governance/check',
        'governance/score',
        'governance/hotspots',
        'governance/duplicates',
        'governance/sql-gates',
      ],
    },
    {
      type: 'category',
      label: 'Commands',
      items: [
        'commands/map',
        'commands/similar',
        'commands/smart',
        'commands/diff',
        'commands/audit',
        'commands/shell',
        'commands/serve',
      ],
    },
    {
      type: 'category',
      label: 'Integrations',
      items: [
        'integrations/claude',
        'integrations/ci-cd',
        'integrations/vscode',
      ],
    },
    {
      type: 'category',
      label: 'Reference',
      items: [
        'configuration',
        'language-support',
        'reference/json-output',
        'reference/exit-codes',
      ],
    },
    'architecture',
  ],
};

export default sidebars;
