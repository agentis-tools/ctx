import type {ReactNode} from 'react';
import clsx from 'clsx';
import Heading from '@theme/Heading';
import styles from './styles.module.css';

type FeatureItem = {
  title: string;
  description: ReactNode;
  icon: string;
};

const FeatureList: FeatureItem[] = [
  {
    title: 'Context Generation',
    icon: '📋',
    description: (
      <>
        Generate perfectly formatted context for LLMs. Smart filtering with .gitignore support,
        170+ built-in ignore patterns, and multiple output formats (XML, Markdown, Plain).
      </>
    ),
  },
  {
    title: 'Code Intelligence',
    icon: '🔍',
    description: (
      <>
        Index your codebase and query it. Find who calls a function, what would break if you
        change something, and visualize call graphs with DOT or Mermaid output.
      </>
    ),
  },
  {
    title: 'Semantic Search',
    icon: '🧠',
    description: (
      <>
        Search code by meaning, not just keywords. Use local embeddings (no API needed)
        or OpenAI for natural language queries like "authentication logic".
      </>
    ),
  },
  {
    title: 'Multi-Language',
    icon: '🌐',
    description: (
      <>
        Full support for Rust, TypeScript, JavaScript, JSX/TSX, Python, Go, Solidity, and YAML.
        Tree-sitter parsing for accurate symbol extraction and relationship tracking.
      </>
    ),
  },
  {
    title: 'Blazingly Fast',
    icon: '⚡',
    description: (
      <>
        Written in Rust for maximum performance. Indexes thousands of files in seconds.
        Incremental updates only reindex what changed.
      </>
    ),
  },
  {
    title: 'Single File Database',
    icon: '📦',
    description: (
      <>
        Everything in one portable SQLite file. No servers, no daemons. Easy to backup,
        share, or delete. Works offline with local embeddings.
      </>
    ),
  },
];

function Feature({title, icon, description}: FeatureItem) {
  return (
    <div className={clsx('col col--4')}>
      <div className="text--center padding-horiz--md margin-bottom--lg">
        <div className={styles.featureIcon}>{icon}</div>
        <Heading as="h3">{title}</Heading>
        <p>{description}</p>
      </div>
    </div>
  );
}

export default function HomepageFeatures(): ReactNode {
  return (
    <section className={styles.features}>
      <div className="container">
        <div className="row">
          {FeatureList.map((props, idx) => (
            <Feature key={idx} {...props} />
          ))}
        </div>
      </div>
    </section>
  );
}
