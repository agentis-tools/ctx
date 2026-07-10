import type {ReactNode} from 'react';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  icon: ReactNode;
  description: ReactNode;
};

const s = {
  fill: 'none',
  stroke: 'currentColor',
  strokeWidth: 1.8,
  strokeLinecap: 'round' as const,
  strokeLinejoin: 'round' as const,
};

const FeatureList: FeatureItem[] = [
  {
    title: 'Right context, fewer tokens',
    icon: (
      <svg viewBox="0 0 24 24" {...s}>
        <path d="M3 5h18l-7 8v6l-4 2v-8z" />
      </svg>
    ),
    description: (
      <>
        <code>ctx smart</code> ranks files by meaning <em>and</em> call-graph relevance, then trims
        them to a token budget. Your agent gets the code that matters, not the whole repo.
      </>
    ),
  },
  {
    title: 'Know what breaks first',
    icon: (
      <svg viewBox="0 0 24 24" {...s}>
        <circle cx="6" cy="6" r="2.4" />
        <circle cx="6" cy="18" r="2.4" />
        <circle cx="18" cy="12" r="2.4" />
        <path d="M8.2 7.4 15.8 11M8.2 16.6 15.8 13" />
      </svg>
    ),
    description: (
      <>
        Impact analysis answers "what depends on this?" before an edit lands — so agents check the
        blast radius instead of missing the caller three hops away.
      </>
    ),
  },
  {
    title: 'Agent-native (MCP + JSON)',
    icon: (
      <svg viewBox="0 0 24 24" {...s}>
        <path d="M9 2v6M15 2v6" />
        <path d="M6 8h12v3a6 6 0 0 1-12 0z" />
        <path d="M12 17v5" />
      </svg>
    ),
    description: (
      <>
        <code>ctx serve --mcp</code> exposes search, impact, and smart context as tools to Claude
        Desktop and other agents. Every command also speaks <code>--output json</code>.
      </>
    ),
  },
  {
    title: 'Structural + semantic',
    icon: (
      <svg viewBox="0 0 24 24" {...s}>
        <circle cx="12" cy="5" r="2.2" />
        <circle cx="5" cy="18" r="2.2" />
        <circle cx="19" cy="18" r="2.2" />
        <path d="M10.5 6.8 6.3 16M13.5 6.8 17.7 16M7 18h10" />
      </svg>
    ),
    description: (
      <>
        Not just grep and not just embeddings. ctx combines tree-sitter call graphs with local
        vector search &mdash; it finds code by <em>meaning</em> and follows the relationships.
      </>
    ),
  },
  {
    title: 'Local, private, offline',
    icon: (
      <svg viewBox="0 0 24 24" {...s}>
        <rect x="4" y="10" width="16" height="10" rx="2" />
        <path d="M8 10V7a4 4 0 0 1 8 0v3" />
      </svg>
    ),
    description: (
      <>
        Written in Rust with local embeddings and a single portable SQLite file. No servers, no API
        keys required &mdash; your source never leaves your machine.
      </>
    ),
  },
  {
    title: 'Multi-language, fast',
    icon: (
      <svg viewBox="0 0 24 24" {...s}>
        <path d="M13 2 4 14h7l-1 8 9-12h-7z" />
      </svg>
    ),
    description: (
      <>
        Rust, TypeScript, JavaScript, JSX/TSX, Python, Go, Solidity, and YAML. Indexes thousands of
        files in seconds and only reindexes what changed.
      </>
    ),
  },
];

function Feature({title, icon, description}: FeatureItem) {
  return (
    <div className={styles.card}>
      <div className={styles.iconWrap}>{icon}</div>
      <Heading as="h3">{title}</Heading>
      <p>{description}</p>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className={styles.sectionHead}>
          <Heading as="h2">One model. Grounded input, governed output.</Heading>
          <p>Precise retrieval and real code intelligence, from one fast local binary.</p>
        </div>
        <div className={styles.grid}>
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
