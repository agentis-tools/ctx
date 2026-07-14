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
        'guides/indexing',
        'guides/using-ctx-with-agents',
      ],
    },
    'context-generation',
    'code-intelligence',
    {
      type: 'category',
      label: 'Cookbook',
      items: [
        'cookbook/cookbook',
        'cookbook/concepts',
        'cookbook/continuous-health',
        'cookbook/pr-governance',
        'cookbook/architecture-drift',
        'cookbook/chronic-hotspots',
        'cookbook/intentional-complexity',
        'cookbook/duplication-trajectories',
        'cookbook/release-health-report',
        'cookbook/unfamiliar-codebase',
        'cookbook/smallest-useful-context',
        'cookbook/find-existing-implementations',
        'cookbook/blast-radius',
        'cookbook/evidence-backed-implementation',
        'cookbook/debug-failing-test',
        'cookbook/review-large-branch',
        'cookbook/untrusted-ci',
        'cookbook/gate-recovery',
      ],
    },
    {
      type: 'category',
      label: 'Governance',
      items: [
        'integrations/quality-gates',
        'commands/check',
        'commands/score',
        'commands/hotspots',
        'commands/duplicates',
        'commands/sql',
        'commands/snapshot',
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
        'commands/harness',
        'commands/self-update',
        'commands/shell',
        'commands/serve',
      ],
    },
    {
      type: 'category',
      label: 'Integrations',
      items: [
        'integrations/claude',
        'integrations/codex',
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
        'json-output',
        'reference/exit-codes',
        'sql-schema',
        'privacy',
      ],
    },
    'architecture',
  ],
};

export default sidebars;
