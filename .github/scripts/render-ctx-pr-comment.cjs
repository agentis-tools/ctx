'use strict';

const MARKER = '<!-- ctx-pr-report -->';
const MAX_COMMENT_LENGTH = 60_000;

function text(value) {
  return String(value ?? '')
    .replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, ' ')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('@', '&#64;')
    .replaceAll('|', '&#124;')
    .replaceAll('`', '&#96;')
    .replace(/[\r\n]+/g, ' ')
    .slice(0, 300);
}

function number(value, fallback = 0) {
  return Number.isFinite(Number(value)) ? Number(value) : fallback;
}

function data(document) {
  return document && typeof document === 'object' && document.data
    ? document.data
    : (document || {});
}

function symbol(value) {
  const item = value || {};
  const name = text(item.qualified_name || item.name || 'unknown');
  const file = text(item.file || 'unknown');
  const line = number(item.line_start, 0);
  return `${name} (${file}${line ? `:${line}` : ''})`.slice(0, 300);
}

function renderComment(report, options = {}) {
  const metadata = report.metadata || {};
  const audit = report.audit || {};
  const stats = data(report.stats);
  const score = data(report.score);
  const metrics = score.metrics || {};
  const duplicates = data(report.duplicates);
  const hotspots = data(report.hotspots);
  const check = data(report.check);
  const map = data(report.map);
  const artifactUrl = text(options.artifactUrl || '');
  const lines = [
    MARKER,
    `<!-- ctx-pr-report-run:${number(options.runId)} -->`,
    '## ctx analysis',
    '',
    `Analyzed \`${text(String(metadata.head_sha || '').slice(0, 12))}\` against \`${text(String(metadata.base_sha || '').slice(0, 12))}\` with ctx ${text(report.stats?.ctx_version || 'unknown')}.`,
    '',
    '| PR delta | Value |',
    '|---|---:|',
    `| Files changed | ${number(metrics.files_changed ?? score.files_changed)} |`,
    `| Complexity delta | ${number(metrics.complexity_delta)} |`,
    `| Fan-out delta | ${number(metrics.fan_out_delta)} |`,
    `| Symbols added / removed | ${number(metrics.symbols_added)} / ${number(metrics.symbols_removed)} |`,
    `| New duplicate pairs | ${number(metrics.new_duplication, (duplicates.pairs || []).length)} |`,
    `| Architecture violations | ${number(metrics.check_violations, check.summary?.violations)} |`,
    `| Whole-codebase audit | ${number(audit.overall_score).toFixed(1)} / 10 |`,
    '',
    '### Repository health',
    '',
    `Indexed **${number(stats.files)} files**, **${number(stats.symbols)} symbols**, **${number(stats.functions)} functions**, and **${number(stats.edges)} relationships**.`,
    '',
    '| Audit category | Score | Issues |',
    '|---|---:|---:|',
  ];

  for (const category of (audit.categories || [])) {
    lines.push(`| ${text(category.name)} | ${number(category.score).toFixed(1)} | ${number(category.issue_count)} |`);
  }

  const issues = (audit.issues || []).filter((issue) => issue.severity !== 'info');
  lines.push('', '<details>', `<summary>Audit findings (${issues.length})</summary>`, '');
  if (issues.length === 0) lines.push('No critical or warning findings.');
  for (const issue of issues.slice(0, 20)) {
    const location = `${text(issue.file)}${issue.line ? `:${number(issue.line)}` : ''}`;
    lines.push(`- **${text(issue.severity)} · ${text(issue.category)} · ${location}** — ${text(issue.message)}`);
  }
  if (issues.length > 20) lines.push(`- … ${issues.length - 20} more in the artifact.`);
  lines.push('', '</details>', '');

  const pairs = duplicates.pairs || [];
  lines.push('<details>', `<summary>New near-duplicate functions (${pairs.length})</summary>`, '');
  if (pairs.length === 0) lines.push('No new near-duplicate functions were found in changed files.');
  for (const pair of pairs.slice(0, 15)) {
    lines.push(`- **${number(pair.similarity).toFixed(3)} similarity:** ${symbol(pair.a)} ↔ ${symbol(pair.b)}`);
  }
  if (pairs.length > 15) lines.push(`- … ${pairs.length - 15} more in the artifact.`);
  lines.push('', '</details>', '');

  const violations = check.violations || [];
  lines.push('<details>', `<summary>Architecture-rule findings (${violations.length})</summary>`, '');
  if (violations.length === 0) lines.push(text(check.note || 'No architecture-rule violations were found.'));
  for (const violation of violations.slice(0, 15)) {
    lines.push(`- **${text(violation.rule_id || violation.rule)}** — ${text(violation.message || violation.reason)}`);
  }
  if (violations.length > 15) lines.push(`- … ${violations.length - 15} more in the artifact.`);
  lines.push('', '</details>', '');

  const entries = hotspots.entries || [];
  lines.push('<details>', `<summary>Changed-code hotspots (${entries.length})</summary>`, '', '| File | Churn | Complexity | Score |', '|---|---:|---:|---:|');
  for (const entry of entries.slice(0, 20)) {
    lines.push(`| ${text(entry.file)} | ${number(entry.commits)} | ${number(entry.complexity)} | ${number(entry.score).toFixed(3)} |`);
  }
  if (entries.length === 0) lines.push('| _No changed file met the churn threshold_ |  |  |  |');
  lines.push('', '</details>', '');

  const mapEntries = map.entries || [];
  lines.push('<details>', `<summary>Architectural map (${mapEntries.length} ranked symbols)</summary>`, '', '```text');
  lines.push(String(map.tree || '')
    .replace(/[\u0000-\u001f\u007f-\u009f\u202a-\u202e\u2066-\u2069]/g, (character) => character === '\n' ? '\n' : ' ')
    .replaceAll('@', '&#64;')
    .replaceAll('```', "''' ")
    .slice(0, 12_000));
  lines.push('```', '');
  for (const entry of mapEntries.slice(0, 40)) {
    lines.push(`- **${text(entry.file)}:${number(entry.line)}** — ${text(entry.signature)}`);
  }
  if (mapEntries.length > 40) lines.push(`- … ${mapEntries.length - 40} more in the artifact.`);
  lines.push('', '</details>', '');

  for (const note of (score.notes || [])) lines.push(`> ${text(note)}`);
  if (score.check_violations_note) lines.push(`> Architecture check: ${text(score.check_violations_note)}`);
  lines.push('', `Full machine-readable results: [workflow artifact](${artifactUrl}).`, '', '_Generated by ctx. This comment is updated on every PR revision._');

  let body = lines.join('\n');
  if (body.length > MAX_COMMENT_LENGTH) {
    const footer = `\n\n</details>\n\n_Comment truncated to fit GitHub's limit; see the [complete workflow artifact](${artifactUrl})._`;
    body = `${body.slice(0, MAX_COMMENT_LENGTH - footer.length)}${footer}`;
  }
  return body;
}

module.exports = { MARKER, MAX_COMMENT_LENGTH, renderComment, text };
