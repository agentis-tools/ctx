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
    'json-output',
    {
      type: 'category',
      label: 'Commands',
      items: [
        'commands/audit',
        'commands/check',
        'commands/score',
        'commands/duplicates',
        'commands/hotspots',
        'commands/similar',
        'commands/map',
        'commands/harness',
        'commands/self-update',
        'commands/diff',
        'commands/smart',
        'commands/shell',
        'commands/serve',
      ],
    },
    {
      type: 'category',
      label: 'Integrations',
      items: [
        'integrations/ci-cd',
        'integrations/quality-gates',
        'integrations/claude',
        'integrations/vscode',
      ],
    },
  ],
};

export default sidebars;
