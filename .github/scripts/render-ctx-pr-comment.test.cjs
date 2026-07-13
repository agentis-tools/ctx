'use strict';

const test = require('node:test');
const assert = require('node:assert/strict');
const { MARKER, MAX_COMMENT_LENGTH, renderComment } = require('./render-ctx-pr-comment.cjs');

function fixture() {
  const envelope = (payload) => ({ ctx_version: '0.3.4', data: payload });
  return {
    metadata: { pr_number: 7, head_sha: 'a'.repeat(40), base_sha: 'b'.repeat(40) },
    audit: { overall_score: 8.25, categories: [{ name: 'complexity', score: 8, issue_count: 1 }], issues: [] },
    stats: envelope({ files: 10, symbols: 20, functions: 12, edges: 30 }),
    score: envelope({ metrics: { files_changed: 2, complexity_delta: 1 }, notes: [] }),
    duplicates: envelope({ pairs: [] }),
    hotspots: envelope({ entries: [] }),
    check: envelope({ summary: { violations: 0 }, violations: [] }),
    map: envelope({ tree: 'repo/\n└── src/', entries: [] }),
  };
}

test('renders a bounded sticky report', () => {
  const body = renderComment(fixture(), { artifactUrl: 'https://example.test/run/1', runId: 42 });
  assert.ok(body.startsWith(MARKER));
  assert.match(body, /ctx-pr-report-run:42/);
  assert.match(body, /Whole-codebase audit \| 8\.3 \/ 10/);
  assert.match(body, /Indexed \*\*10 files\*\*/);
  assert.ok(body.length <= MAX_COMMENT_LENGTH);
});

test('neutralizes artifact-controlled markdown and mentions', () => {
  const report = fixture();
  report.audit.issues = [{ severity: 'warning', category: 'x|y', file: '<img>', message: '@team `boom`' }];
  const body = renderComment(report, { artifactUrl: 'https://example.test' });
  assert.doesNotMatch(body, /<img>/);
  assert.doesNotMatch(body, /@team/);
  assert.doesNotMatch(body, /`boom`/);
  assert.match(body, /&lt;img&gt;/);
  assert.match(body, /&#64;team/);
});

test('neutralizes map fence breaks, mentions, controls, and bidi overrides', () => {
  const report = fixture();
  report.map.data.tree = 'safe```\n@team\u0000\u202emore';
  const body = renderComment(report, { artifactUrl: 'https://example.test' });
  assert.doesNotMatch(body, /safe```/);
  assert.doesNotMatch(body, /@team|\u0000|\u202e/);
  assert.match(body, /&#64;team/);
});

test('truncates oversized reports below the GitHub comment limit', () => {
  const report = fixture();
  report.map.tree = 'x'.repeat(MAX_COMMENT_LENGTH * 2);
  report.audit.issues = Array.from({ length: 100 }, (_, i) => ({
    severity: 'warning', category: 'large', file: `file-${i}`, message: 'y'.repeat(2000),
  }));
  const body = renderComment(report, { artifactUrl: 'https://example.test' });
  assert.ok(body.length <= MAX_COMMENT_LENGTH);
  assert.match(body, /truncated|more in the artifact/);
});
