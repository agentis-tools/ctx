import type {SidebarsConfig} from '@docusaurus/plugin-content-docs';

const sidebars: SidebarsConfig = {
  docsSidebar: [
    'intro',
    'getting-started',
    'context-generation',
    'code-intelligence',
    'configuration',
    'language-support',
    'architecture',
    'sql-schema',
    {
      type: 'category',
      label: 'Commands',
      items: [
        'commands/audit',
        'commands/diff',
        'commands/smart',
        'commands/sql',
        'commands/shell',
        'commands/serve',
      ],
    },
    {
      type: 'category',
      label: 'Integrations',
      items: [
        'integrations/ci-cd',
        'integrations/claude',
        'integrations/vscode',
      ],
    },
  ],
};

export default sidebars;
