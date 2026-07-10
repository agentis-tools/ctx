import React, {isValidElement, useState, type ReactNode} from 'react';
import {createHighlighterCoreSync, type HighlighterCore} from 'shiki/core';
import {createJavaScriptRegexEngine} from 'shiki/engine/javascript';
import minLight from '@shikijs/themes/min-light';
import minDark from '@shikijs/themes/min-dark';
import bash from '@shikijs/langs/bash';
import rust from '@shikijs/langs/rust';
import typescript from '@shikijs/langs/typescript';
import javascript from '@shikijs/langs/javascript';
import tsx from '@shikijs/langs/tsx';
import python from '@shikijs/langs/python';
import go from '@shikijs/langs/go';
import json from '@shikijs/langs/json';
import yaml from '@shikijs/langs/yaml';
import toml from '@shikijs/langs/toml';
import sql from '@shikijs/langs/sql';
import diff from '@shikijs/langs/diff';
import markdown from '@shikijs/langs/markdown';

import styles from './styles.module.css';

// A single synchronous highlighter shared across all code blocks. Created once at
// module load (works during SSG and in the browser) so rendering stays synchronous.
const highlighter: HighlighterCore = createHighlighterCoreSync({
  themes: [minLight, minDark],
  langs: [bash, rust, typescript, javascript, tsx, python, go, json, yaml, toml, sql, diff, markdown],
  engine: createJavaScriptRegexEngine(),
});

const loadedLangs = new Set(highlighter.getLoadedLanguages());

type Props = {
  children?: ReactNode;
  className?: string;
  language?: string;
  title?: string;
  metastring?: string;
};

function toCodeString(value: ReactNode): string {
  if (typeof value === 'string') return value;
  if (Array.isArray(value)) return value.map(toCodeString).join('');
  return value == null ? '' : String(value);
}

function extract(props: Props): {code: string; lang: string} {
  const {children, className, language} = props;
  // JSX usage: <CodeBlock language="bash">{"..."}</CodeBlock>
  if (typeof children === 'string') {
    const lang = language || className?.replace(/language-/, '') || 'text';
    return {code: children, lang};
  }
  // MDX usage: children is a <code className="language-x"> element
  if (isValidElement(children)) {
    const p = (children.props ?? {}) as {className?: string; children?: ReactNode};
    const lang = p.className?.match(/language-([\w-]+)/)?.[1] || language || 'text';
    return {code: toCodeString(p.children), lang};
  }
  return {code: toCodeString(children), lang: language || 'text'};
}

export default function CodeBlock(props: Props): ReactNode {
  const {title} = props;
  const {code, lang} = extract(props);
  const clean = code.replace(/\n$/, '');
  const useLang = loadedLangs.has(lang) ? lang : 'text';

  const html = highlighter.codeToHtml(clean, {
    lang: useLang,
    themes: {light: 'min-light', dark: 'min-dark'},
    defaultColor: false,
  });

  const [copied, setCopied] = useState(false);
  const onCopy = () => {
    navigator.clipboard?.writeText(clean).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    });
  };

  return (
    <div className={styles.wrapper}>
      {title ? <div className={styles.title}>{title}</div> : null}
      <button
        type="button"
        className={styles.copyButton}
        onClick={onCopy}
        aria-label="Copy code to clipboard">
        {copied ? 'Copied' : 'Copy'}
      </button>
      <div className={styles.code} dangerouslySetInnerHTML={{__html: html}} />
    </div>
  );
}
