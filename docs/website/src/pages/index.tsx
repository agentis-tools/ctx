import type {ReactNode} from 'react';
import clsx from 'clsx';
import Link from '@docusaurus/Link';
import Layout from '@theme/Layout';
import CodeBlock from '@theme/CodeBlock';
import HomepageFeatures from '@site/src/components/HomepageFeatures';
import Heading from '@theme/Heading';

import styles from './index.module.css';

function HomepageHeader() {
  return (
    <header className={styles.hero}>
      <div className="container">
        <div className={styles.eyebrow}>ctx &middot; a queryable model of your codebase</div>
        <Heading as="h1" className={styles.heroTitle}>
          The local quality authority for <span className={styles.accent}>AI-written code</span>
        </Heading>
        <p className={styles.heroSubtitle}>
          ctx is a queryable model of your codebase that <strong>grounds</strong> your agent and{' '}
          <strong>gates</strong> its output &mdash; every turn. It hands your agent a map before it
          starts, shows the blast radius of every edit, and enforces your rules as deterministic
          gates. Locally.
        </p>
        <div className={styles.install}>
          <CodeBlock language="bash">cargo install agentis-ctx</CodeBlock>
        </div>
        <div className={styles.buttons}>
          <Link className="button button--primary button--lg" to="/docs/getting-started">
            Get started
          </Link>
          <Link className="button button--outline button--secondary button--lg" to="/docs/why-ctx">
            Why ctx?
          </Link>
          <Link
            className="button button--outline button--secondary button--lg"
            href="https://github.com/agentis-tools/ctx">
            GitHub
          </Link>
        </div>
      </div>
    </header>
  );
}

function ProofBand(): ReactNode {
  return (
    <section className={styles.statBand}>
      <div className="container">
        <div className={styles.stats}>
          <div className={styles.stat}>
            <div className={styles.statNum}>~27&times;</div>
            <div className={styles.statLabel}>
              smaller context &mdash; 233k &rarr; ~8.7k tokens for a task with <code>ctx smart</code>
            </div>
          </div>
          <div className={styles.stat}>
            <div className={styles.statNum}>0.36s</div>
            <div className={styles.statLabel}>
              to index 870 symbols and 5,463 call edges (measured on this repo)
            </div>
          </div>
          <div className={styles.stat}>
            <div className={styles.statNum}>100%</div>
            <div className={styles.statLabel}>
              local &mdash; one SQLite file, offline embeddings, your code never leaves your machine
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

function QuickStart(): ReactNode {
  return (
    <section className={styles.quickStart}>
      <div className="container">
        <Heading as="h2" className={styles.sectionTitle}>
          Ground and govern every change
        </Heading>
        <p className={styles.qsIntro}>
          One world model, two jobs: feed the model the right context going in, and guardrail what it
          changes coming out.
        </p>
        <div className="row">
          <div className="col col--6">
            <div className={styles.codeCard}>
              <div className={styles.codeCardHeader}>Ground &mdash; the right context, in</div>
              <CodeBlock language="bash">{`# Build the world model once
ctx index

# Task-scoped, token-budgeted context
ctx smart "add rate limiting" --max-tokens 8000

# Or just the git changes, with call-graph context
ctx diff --summary`}</CodeBlock>
            </div>
          </div>
          <div className="col col--6">
            <div className={styles.codeCard}>
              <div className={styles.codeCardHeader}>Govern &mdash; guardrails, on what changes</div>
              <CodeBlock language="bash">{`# Enforce your architecture rules on the change
ctx check --against origin/main

# One gate for the whole change (exit 1 = not done)
ctx score --fail-on "check_violations>0,new_duplication>0"

# Auto-gate every agent edit via Claude Code hooks
ctx harness init --target claude`}</CodeBlock>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

export default function Home(): ReactNode {
  return (
    <Layout
      title="The local quality authority for AI-written code"
      description="ctx is the local quality authority for AI-written code — a queryable model of your codebase that grounds your agent and gates its output every turn, enforcing your architecture rules and quality thresholds as deterministic gates, locally, in the agent's loop.">
      <HomepageHeader />
      <main>
        <ProofBand />
        <HomepageFeatures />
        <QuickStart />
      </main>
    </Layout>
  );
}
