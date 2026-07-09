import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import useDocusaurusContext from '@docusaurus/useDocusaurusContext';
import Layout from '@theme/Layout';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
import Heading from '@theme/Heading';

import styles from './index.module.css';

function HomepageHeader() {
  const {siteConfig} = useDocusaurusContext();
  return (
    <header className={clsx('hero hero--primary', styles.heroBanner)}>
      <div className="container">
        <Heading as="h1" className="hero__title">
          {siteConfig.title}
        </Heading>
        <p className="hero__subtitle">{siteConfig.tagline}</p>
        <div className={styles.buttons}>
          <Link
            className="button button--secondary button--lg"
            to="/docs/">
            Get Started
          </Link>
          <Link
            className="button button--outline button--secondary button--lg"
            style={{marginLeft: '1rem'}}
            href="https://github.com/saldestechnology/ctx">
            GitHub
          </Link>
        </div>
      </div>
    </header>
  );
}

function QuickStart(): ReactNode {
  return (
    <section className={styles.quickStart}>
      <div className="container">
        <Heading as="h2" className="text--center margin-bottom--lg">
          Quick Start
        </Heading>
        <div className="row">
          <div className="col col--6">
            <div className={styles.codeBlock}>
              <div className={styles.codeHeader}>Context Generation</div>
              <pre>
                <code>{`# Generate context for an LLM
ctx src/ | pbcopy

# Specific file patterns
ctx "src/**/*.ts" "lib/**/*.rs"

# Markdown format
ctx --format markdown src/`}</code>
              </pre>
            </div>
          </div>
          <div className="col col--6">
            <div className={styles.codeBlock}>
              <div className={styles.codeHeader}>Code Intelligence</div>
              <pre>
                <code>{`# Build the index
ctx index

# Search for symbols
ctx search "handleRequest"

# Find callers of a function
ctx query callers authenticate

# Impact analysis
ctx query impact validateToken`}</code>
              </pre>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  const {siteConfig} = useDocusaurusContext();
  return (
    <Layout
      title="AI-ready context from your codebase"
      description="Fast CLI tool that generates AI-ready context from codebases with built-in code intelligence for understanding symbol relationships and call graphs.">
      <HomepageHeader />
      <main>
        <HomepageFeatures />
        <QuickStart />
      </main>
    </Layout>
  );
}
