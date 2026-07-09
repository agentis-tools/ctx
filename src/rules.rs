//! Architecture rules for `ctx check`.
//!
//! Rules live in `.ctx/rules.toml` and are checked against the code
//! intelligence index. The file declares named *layers* (glob patterns over
//! indexed file paths) plus four kinds of rules:
//!
//! - `[[rules.forbidden]]` -- layer A must not depend on layer B
//! - `[[rules.allowed_dependents]]` -- only the listed layers may depend on a layer
//! - `[[rules.limit]]` -- metric thresholds (fan-in/fan-out/complexity/file symbols)
//! - `[[rules.no_new_dependents]]` -- frozen paths that must not gain callers
//!
//! This module contains the serde model, glob compilation/validation, and
//! pure evaluation functions over a pre-built list of [`FileDep`]s, so the
//! rule engine is unit-testable without a database.

use std::collections::{BTreeMap, HashSet};

use globset::{Glob, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::db::{FileComplexity, SymbolMetrics};
use crate::error::{CtxError, Result};
use crate::json::SymbolRef;

/// Default location of the rules file, relative to the project root.
pub const DEFAULT_RULES_PATH: &str = ".ctx/rules.toml";

/// The rules file format version this build understands.
pub const SUPPORTED_VERSION: u32 = 1;

// ============================================================================
// Serde model
// ============================================================================

/// Parsed `.ctx/rules.toml`.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RulesFile {
    /// Format version (currently always `1`).
    #[serde(default = "default_version")]
    pub version: u32,

    /// Layer name -> glob patterns over indexed file paths.
    #[serde(default)]
    pub layers: BTreeMap<String, Vec<String>>,

    /// The rule groups.
    #[serde(default)]
    pub rules: RuleGroups,
}

fn default_version() -> u32 {
    SUPPORTED_VERSION
}

/// The `[rules]` table.
#[derive(Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleGroups {
    #[serde(default)]
    pub forbidden: Vec<ForbiddenRule>,

    #[serde(default)]
    pub allowed_dependents: Vec<AllowedDependentsRule>,

    #[serde(default)]
    pub limit: Vec<LimitRule>,

    #[serde(default)]
    pub no_new_dependents: Vec<NoNewDependentsRule>,
}

/// `[[rules.forbidden]]`: dependencies from `from` to `to` are violations.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ForbiddenRule {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `[[rules.allowed_dependents]]`: only layers in `only` may depend on `layer`.
///
/// Files that belong to no layer are exempt: per the layer model, unlayered
/// files are unconstrained, which keeps incremental rollout sane (you can
/// start with a couple of layers without instantly flagging the rest of the
/// codebase).
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedDependentsRule {
    pub layer: String,
    pub only: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `[[rules.limit]]`: a metric threshold over symbols or files.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LimitRule {
    pub metric: Metric,
    #[serde(default)]
    pub scope: Scope,
    pub max: i64,
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// `[[rules.no_new_dependents]]`: frozen paths that must not gain dependents.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NoNewDependentsRule {
    pub paths: Vec<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Metric checked by a `[[rules.limit]]` rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Metric {
    FanIn,
    FanOut,
    Complexity,
    FileSymbols,
}

impl Metric {
    pub fn as_str(self) -> &'static str {
        match self {
            Metric::FanIn => "fan_in",
            Metric::FanOut => "fan_out",
            Metric::Complexity => "complexity",
            Metric::FileSymbols => "file_symbols",
        }
    }
}

/// Scope of a `[[rules.limit]]` rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    #[default]
    Symbol,
    File,
}

impl Scope {
    pub fn as_str(self) -> &'static str {
        match self {
            Scope::Symbol => "symbol",
            Scope::File => "file",
        }
    }
}

// ============================================================================
// Compilation and validation
// ============================================================================

/// A [`RulesFile`] with all glob patterns compiled.
#[derive(Debug)]
pub struct CompiledRules {
    pub file: RulesFile,
    /// Layer name + compiled globs, in declaration (BTreeMap) order.
    layer_globs: Vec<(String, GlobSet)>,
    /// Compiled `exclude` globs, parallel to `file.rules.limit`.
    limit_excludes: Vec<GlobSet>,
    /// Compiled `paths` globs, parallel to `file.rules.no_new_dependents`.
    frozen_paths: Vec<GlobSet>,
}

impl CompiledRules {
    /// Compile all glob patterns in `file`.
    ///
    /// Fails on unsupported versions and invalid glob patterns.
    pub fn compile(file: RulesFile) -> Result<Self> {
        if file.version != SUPPORTED_VERSION {
            return Err(CtxError::Other(format!(
                "unsupported rules version {} (this build supports version {})",
                file.version, SUPPORTED_VERSION
            )));
        }

        let mut layer_globs = Vec::new();
        for (name, patterns) in &file.layers {
            let set = build_globset(patterns, &format!("layer '{}'", name))?;
            layer_globs.push((name.clone(), set));
        }

        let mut limit_excludes = Vec::new();
        for (i, rule) in file.rules.limit.iter().enumerate() {
            limit_excludes.push(build_globset(
                &rule.exclude,
                &format!("rules.limit[{}].exclude", i),
            )?);
        }

        let mut frozen_paths = Vec::new();
        for (i, rule) in file.rules.no_new_dependents.iter().enumerate() {
            frozen_paths.push(build_globset(
                &rule.paths,
                &format!("rules.no_new_dependents[{}].paths", i),
            )?);
        }

        Ok(CompiledRules {
            file,
            layer_globs,
            limit_excludes,
            frozen_paths,
        })
    }

    /// The layer `path` belongs to, or `None` if it matches no layer.
    ///
    /// [`Self::validate`] guarantees indexed files match at most one layer.
    pub fn layer_of(&self, path: &str) -> Option<&str> {
        self.layer_globs
            .iter()
            .find(|(_, set)| set.is_match(path))
            .map(|(name, _)| name.as_str())
    }

    /// All layers `path` belongs to (used for overlap detection).
    pub fn layers_of(&self, path: &str) -> Vec<&str> {
        self.layer_globs
            .iter()
            .filter(|(_, set)| set.is_match(path))
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Validate the rules against the set of indexed files.
    ///
    /// Errors on:
    /// - rules referencing undeclared layer names
    /// - an indexed file matching two or more layers
    /// - `metric = "file_symbols"` combined with `scope = "symbol"`
    pub fn validate(&self, indexed_files: &[String]) -> Result<()> {
        // Unknown layer references.
        for rule in &self.file.rules.forbidden {
            self.require_layer(&rule.from, "rules.forbidden.from")?;
            self.require_layer(&rule.to, "rules.forbidden.to")?;
        }
        for rule in &self.file.rules.allowed_dependents {
            self.require_layer(&rule.layer, "rules.allowed_dependents.layer")?;
            for only in &rule.only {
                self.require_layer(only, "rules.allowed_dependents.only")?;
            }
        }

        // file_symbols is a per-file metric; symbol scope makes no sense.
        for rule in &self.file.rules.limit {
            if rule.metric == Metric::FileSymbols && rule.scope == Scope::Symbol {
                return Err(CtxError::Other(
                    "rules.limit: metric \"file_symbols\" requires scope = \"file\" \
                     (it counts symbols per file and has no per-symbol value)"
                        .to_string(),
                ));
            }
        }

        // Overlapping layers: every indexed file must match at most one layer.
        for path in indexed_files {
            let layers = self.layers_of(path);
            if layers.len() >= 2 {
                return Err(CtxError::Other(format!(
                    "layer overlap: file '{}' matches layers [{}]; \
                     each file may belong to at most one layer",
                    path,
                    layers.join(", ")
                )));
            }
        }

        Ok(())
    }

    fn require_layer(&self, name: &str, context: &str) -> Result<()> {
        if self.file.layers.contains_key(name) {
            Ok(())
        } else {
            let known: Vec<&str> = self.file.layers.keys().map(|k| k.as_str()).collect();
            Err(CtxError::Other(format!(
                "{}: unknown layer '{}' (declared layers: [{}])",
                context,
                name,
                known.join(", ")
            )))
        }
    }

    /// Compiled `exclude` globset for `file.rules.limit[i]`.
    pub fn limit_exclude(&self, i: usize) -> &GlobSet {
        &self.limit_excludes[i]
    }

    /// Compiled `paths` globset for `file.rules.no_new_dependents[i]`.
    pub fn frozen_path(&self, i: usize) -> &GlobSet {
        &self.frozen_paths[i]
    }

    /// Count of indexed files per layer, in declaration order.
    pub fn layer_file_counts(&self, indexed_files: &[String]) -> Vec<(String, usize)> {
        self.layer_globs
            .iter()
            .map(|(name, set)| {
                let n = indexed_files.iter().filter(|f| set.is_match(f)).count();
                (name.clone(), n)
            })
            .collect()
    }
}

fn build_globset(patterns: &[String], what: &str) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        let glob = Glob::new(pattern).map_err(|e| {
            CtxError::Other(format!("invalid glob '{}' in {}: {}", pattern, what, e))
        })?;
        builder.add(glob);
    }
    builder
        .build()
        .map_err(|e| CtxError::Other(format!("failed to build globs for {}: {}", what, e)))
}

// ============================================================================
// Dependency and violation model
// ============================================================================

/// One endpoint of a dependency: either a resolved symbol or a whole file
/// (file-level endpoints come from import resolution).
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum Endpoint {
    Symbol(SymbolRef),
    File { file: String },
}

impl Endpoint {
    /// The file this endpoint lives in.
    pub fn file(&self) -> &str {
        match self {
            Endpoint::Symbol(s) => &s.file,
            Endpoint::File { file } => file,
        }
    }
}

/// A cross-file dependency (call/implements/extends/uses edge or resolved import).
#[derive(Debug, Clone)]
pub struct FileDep {
    pub from: Endpoint,
    pub to: Endpoint,
    /// Edge kind (`calls`, `implements`, `extends`, `uses`) or `import`.
    pub kind: String,
    /// Line in the `from` file where the dependency occurs (unknown for
    /// imports read from module metadata).
    pub line: Option<i64>,
}

/// Which rule group a violation belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleKind {
    Forbidden,
    AllowedDependents,
    Limit,
    NoNewDependents,
}

impl RuleKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleKind::Forbidden => "forbidden",
            RuleKind::AllowedDependents => "allowed_dependents",
            RuleKind::Limit => "limit",
            RuleKind::NoNewDependents => "no_new_dependents",
        }
    }
}

/// One rule violation.
///
/// Dependency violations (`forbidden`, `allowed_dependents`,
/// `no_new_dependents`) carry `from`/`to` endpoints; `limit` violations carry
/// a `subject` endpoint plus the metric fields.
#[derive(Debug, Clone, Serialize)]
pub struct Violation {
    pub rule: RuleKind,
    /// Stable identifier of the specific rule instance, used for grouping
    /// (e.g. `forbidden: domain -> infrastructure`).
    pub rule_id: String,
    pub reason: String,
    /// One-line human description of the violating dependency or metric.
    pub message: String,
    /// Primary file (`from` side for dependency rules, subject file for limits).
    pub file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<Endpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<Endpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<Endpoint>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<Metric>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

impl Violation {
    fn dep(rule: RuleKind, rule_id: String, reason: String, dep: &FileDep) -> Violation {
        let location = match dep.line {
            Some(line) => format!("{}:{}", dep.from.file(), line),
            None => dep.from.file().to_string(),
        };
        let target = match &dep.to {
            Endpoint::Symbol(s) => format!("{} [{} {}]", s.file, dep.kind, s.name),
            Endpoint::File { file } => format!("{} [{}]", file, dep.kind),
        };
        let message = format!("{} -> {}", location, target);
        Violation {
            rule,
            rule_id,
            reason,
            message,
            file: dep.from.file().to_string(),
            line: dep.line,
            from: Some(dep.from.clone()),
            to: Some(dep.to.clone()),
            subject: None,
            metric: None,
            scope: None,
            value: None,
            max: None,
        }
    }

    /// Files referenced by this violation (for `--against` filtering).
    fn endpoint_files(&self) -> Vec<&str> {
        let mut files = vec![self.file.as_str()];
        for endpoint in [&self.from, &self.to, &self.subject].into_iter().flatten() {
            files.push(endpoint.file());
        }
        files
    }
}

// ============================================================================
// Evaluation
// ============================================================================

/// Evaluate all rules and return the violations.
///
/// `changed` is the `--against REF` change set. When present it acts as a
/// filter (documented v1 approximation):
/// - `no_new_dependents`: only inbound deps whose *source* file changed count
///   (that is what "new dependent" means); without `--against` **all** inbound
///   deps are reported so users can see the current state.
/// - all other rules: a violation is kept when at least one endpoint's file
///   is in the change set.
pub fn evaluate(
    compiled: &CompiledRules,
    deps: &[FileDep],
    symbol_metrics: &[SymbolMetrics],
    file_complexity: &[FileComplexity],
    changed: Option<&HashSet<String>>,
) -> Vec<Violation> {
    let mut violations = Vec::new();

    violations.extend(eval_forbidden(compiled, deps));
    violations.extend(eval_allowed_dependents(compiled, deps));
    violations.extend(eval_limits(compiled, symbol_metrics, file_complexity));
    violations.extend(eval_no_new_dependents(compiled, deps, changed));

    if let Some(changed) = changed {
        violations.retain(|v| {
            // no_new_dependents already applied its own (source-side) filter.
            v.rule == RuleKind::NoNewDependents
                || v.endpoint_files().iter().any(|f| changed.contains(*f))
        });
    }

    violations
}

/// C2: dependency edges from `from`-layer files into `to`-layer files.
fn eval_forbidden(compiled: &CompiledRules, deps: &[FileDep]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rule in &compiled.file.rules.forbidden {
        let rule_id = format!("forbidden: {} -> {}", rule.from, rule.to);
        let reason = rule
            .reason
            .clone()
            .unwrap_or_else(|| format!("layer '{}' must not depend on '{}'", rule.from, rule.to));
        for dep in deps {
            if compiled.layer_of(dep.from.file()) == Some(rule.from.as_str())
                && compiled.layer_of(dep.to.file()) == Some(rule.to.as_str())
            {
                violations.push(Violation::dep(
                    RuleKind::Forbidden,
                    rule_id.clone(),
                    reason.clone(),
                    dep,
                ));
            }
        }
    }
    violations
}

/// C3: inbound deps into `layer` from a file whose layer is not in `only`.
///
/// Files in no layer are exempt (unlayered files are unconstrained, matching
/// the C1 layer model), and a layer may always depend on itself.
fn eval_allowed_dependents(compiled: &CompiledRules, deps: &[FileDep]) -> Vec<Violation> {
    let mut violations = Vec::new();
    for rule in &compiled.file.rules.allowed_dependents {
        let rule_id = format!(
            "allowed_dependents: {} only [{}]",
            rule.layer,
            rule.only.join(", ")
        );
        let reason = rule.reason.clone().unwrap_or_else(|| {
            format!(
                "only [{}] may depend on layer '{}'",
                rule.only.join(", "),
                rule.layer
            )
        });
        for dep in deps {
            if compiled.layer_of(dep.to.file()) != Some(rule.layer.as_str()) {
                continue;
            }
            let Some(from_layer) = compiled.layer_of(dep.from.file()) else {
                continue; // unlayered files are unconstrained
            };
            if from_layer == rule.layer || rule.only.iter().any(|o| o == from_layer) {
                continue;
            }
            violations.push(Violation::dep(
                RuleKind::AllowedDependents,
                rule_id.clone(),
                format!("{} (found dependent in layer '{}')", reason, from_layer),
                dep,
            ));
        }
    }
    violations
}

/// C4: metric thresholds over symbols or files.
fn eval_limits(
    compiled: &CompiledRules,
    symbol_metrics: &[SymbolMetrics],
    file_complexity: &[FileComplexity],
) -> Vec<Violation> {
    let mut violations = Vec::new();

    for (i, rule) in compiled.file.rules.limit.iter().enumerate() {
        let exclude = compiled.limit_exclude(i);
        let rule_id = format!(
            "limit: {} <= {} ({})",
            rule.metric.as_str(),
            rule.max,
            rule.scope.as_str()
        );

        match rule.scope {
            Scope::Symbol => {
                // metric != file_symbols is guaranteed by validate().
                for m in symbol_metrics {
                    if exclude.is_match(&m.file_path) {
                        continue;
                    }
                    let value = symbol_metric_value(rule.metric, m);
                    if value > rule.max {
                        violations.push(limit_violation(
                            rule,
                            rule_id.clone(),
                            Endpoint::Symbol(symbol_ref_of(m)),
                            format!("{}:{}", m.file_path, m.line_start),
                            m.file_path.clone(),
                            Some(m.line_start),
                            Some(&m.name),
                            value,
                        ));
                    }
                }
            }
            Scope::File => {
                // file_symbols comes straight from per-file symbol counts;
                // other metrics are summed over the file's functions/methods.
                let per_file: BTreeMap<String, i64> = if rule.metric == Metric::FileSymbols {
                    file_complexity
                        .iter()
                        .map(|f| (f.file_path.clone(), f.symbol_count))
                        .collect()
                } else {
                    let mut sums: BTreeMap<String, i64> = BTreeMap::new();
                    for m in symbol_metrics {
                        *sums.entry(m.file_path.clone()).or_insert(0) +=
                            symbol_metric_value(rule.metric, m);
                    }
                    sums
                };

                for (file, value) in per_file {
                    if exclude.is_match(&file) || value <= rule.max {
                        continue;
                    }
                    violations.push(limit_violation(
                        rule,
                        rule_id.clone(),
                        Endpoint::File { file: file.clone() },
                        file.clone(),
                        file,
                        None,
                        None,
                        value,
                    ));
                }
            }
        }
    }

    violations
}

fn symbol_metric_value(metric: Metric, m: &SymbolMetrics) -> i64 {
    match metric {
        Metric::FanIn => m.fan_in,
        Metric::FanOut => m.fan_out,
        Metric::Complexity => m.complexity,
        // Unreachable for symbol scope (rejected by validate()); a file-level
        // sum of file_symbols never consults symbol metrics.
        Metric::FileSymbols => 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn limit_violation(
    rule: &LimitRule,
    rule_id: String,
    subject: Endpoint,
    location: String,
    file: String,
    line: Option<i64>,
    name: Option<&str>,
    value: i64,
) -> Violation {
    let what = match name {
        Some(name) => format!("{} ({})", location, name),
        None => location,
    };
    Violation {
        rule: RuleKind::Limit,
        rule_id,
        reason: format!(
            "{} {} exceeds max {}",
            rule.metric.as_str(),
            value,
            rule.max
        ),
        message: format!(
            "{}: {} {} exceeds max {}",
            what,
            rule.metric.as_str(),
            value,
            rule.max
        ),
        file,
        line,
        from: None,
        to: None,
        subject: Some(subject),
        metric: Some(rule.metric),
        scope: Some(rule.scope),
        value: Some(value),
        max: Some(rule.max),
    }
}

fn symbol_ref_of(m: &SymbolMetrics) -> SymbolRef {
    SymbolRef {
        name: m.name.clone(),
        qualified_name: m.qualified_name.clone(),
        kind: m.kind.clone(),
        file: m.file_path.clone(),
        line_start: m.line_start,
        line_end: m.line_end,
    }
}

/// C5: inbound deps into frozen `paths` from outside those paths.
///
/// With `--against` (`changed` is `Some`), only deps whose source file
/// changed are violations ("new dependents"). Without it, **all** inbound
/// deps are reported so users can see the current state.
fn eval_no_new_dependents(
    compiled: &CompiledRules,
    deps: &[FileDep],
    changed: Option<&HashSet<String>>,
) -> Vec<Violation> {
    let mut violations = Vec::new();
    for (i, rule) in compiled.file.rules.no_new_dependents.iter().enumerate() {
        let paths = compiled.frozen_path(i);
        let rule_id = format!("no_new_dependents: [{}]", rule.paths.join(", "));
        let reason = rule
            .reason
            .clone()
            .unwrap_or_else(|| format!("[{}] must not gain new dependents", rule.paths.join(", ")));
        for dep in deps {
            if !paths.is_match(dep.to.file()) || paths.is_match(dep.from.file()) {
                continue;
            }
            if let Some(changed) = changed {
                if !changed.contains(dep.from.file()) {
                    continue;
                }
            }
            violations.push(Violation::dep(
                RuleKind::NoNewDependents,
                rule_id.clone(),
                reason.clone(),
                dep,
            ));
        }
    }
    violations
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const FULL_RULES: &str = r#"
version = 1

[layers]
domain         = ["src/domain/**"]
application    = ["src/app/**"]
infrastructure = ["src/infra/**", "src/db/**"]

[[rules.forbidden]]
from   = "domain"
to     = "infrastructure"
reason = "Domain layer must stay persistence-agnostic"

[[rules.allowed_dependents]]
layer = "infrastructure"
only  = ["application"]

[[rules.limit]]
metric  = "fan_in"
scope   = "symbol"
max     = 25
exclude = ["src/core/**"]

[[rules.limit]]
metric = "file_symbols"
scope  = "file"
max    = 50

[[rules.no_new_dependents]]
paths  = ["src/legacy/**"]
reason = "Legacy module is frozen; do not add new callers"
"#;

    fn compile(toml_src: &str) -> CompiledRules {
        let file: RulesFile = toml::from_str(toml_src).unwrap();
        CompiledRules::compile(file).unwrap()
    }

    fn file_dep(from: &str, to: &str) -> FileDep {
        FileDep {
            from: Endpoint::File {
                file: from.to_string(),
            },
            to: Endpoint::File {
                file: to.to_string(),
            },
            kind: "import".to_string(),
            line: None,
        }
    }

    fn symbol_dep(from_file: &str, from_name: &str, to_file: &str, to_name: &str) -> FileDep {
        let sym = |file: &str, name: &str| {
            Endpoint::Symbol(SymbolRef {
                name: name.to_string(),
                qualified_name: None,
                kind: "function".to_string(),
                file: file.to_string(),
                line_start: 1,
                line_end: 2,
            })
        };
        FileDep {
            from: sym(from_file, from_name),
            to: sym(to_file, to_name),
            kind: "calls".to_string(),
            line: Some(1),
        }
    }

    fn metrics(
        name: &str,
        file: &str,
        fan_in: i64,
        fan_out: i64,
        complexity: i64,
    ) -> SymbolMetrics {
        SymbolMetrics {
            id: format!("{}::{}", file, name),
            name: name.to_string(),
            qualified_name: None,
            kind: "function".to_string(),
            file_path: file.to_string(),
            line_start: 10,
            line_end: 20,
            fan_in,
            fan_out,
            complexity,
        }
    }

    // ---------- parsing ----------

    #[test]
    fn test_parse_full_rules_file() {
        let file: RulesFile = toml::from_str(FULL_RULES).unwrap();
        assert_eq!(file.version, 1);
        assert_eq!(file.layers.len(), 3);
        assert_eq!(file.layers["infrastructure"].len(), 2);
        assert_eq!(file.rules.forbidden.len(), 1);
        assert_eq!(file.rules.forbidden[0].from, "domain");
        assert_eq!(file.rules.allowed_dependents.len(), 1);
        assert_eq!(file.rules.limit.len(), 2);
        assert_eq!(file.rules.limit[0].metric, Metric::FanIn);
        assert_eq!(file.rules.limit[0].scope, Scope::Symbol);
        assert_eq!(file.rules.limit[0].max, 25);
        assert_eq!(file.rules.no_new_dependents.len(), 1);
    }

    #[test]
    fn test_parse_defaults() {
        // Empty file: everything defaults.
        let file: RulesFile = toml::from_str("").unwrap();
        assert_eq!(file.version, 1);
        assert!(file.layers.is_empty());
        assert!(file.rules.forbidden.is_empty());

        // Scope defaults to symbol; exclude defaults to empty.
        let file: RulesFile = toml::from_str(
            r#"
            [[rules.limit]]
            metric = "fan_out"
            max = 10
            "#,
        )
        .unwrap();
        assert_eq!(file.rules.limit[0].scope, Scope::Symbol);
        assert!(file.rules.limit[0].exclude.is_empty());
    }

    #[test]
    fn test_parse_rejects_unknown_fields_and_bad_values() {
        // Typo in a rule field.
        let err = toml::from_str::<RulesFile>(
            r#"
            [[rules.forbidden]]
            form = "domain"
            to = "infrastructure"
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("form"), "err: {}", err);

        // Unknown rule group.
        assert!(toml::from_str::<RulesFile>("[[rules.forbid]]\nx = 1").is_err());

        // Invalid metric value.
        let err = toml::from_str::<RulesFile>(
            r#"
            [[rules.limit]]
            metric = "fan_inn"
            max = 5
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("fan_inn"), "err: {}", err);

        // Invalid TOML syntax.
        assert!(toml::from_str::<RulesFile>("[layers\ndomain = 1").is_err());
    }

    #[test]
    fn test_compile_rejects_unsupported_version() {
        let file: RulesFile = toml::from_str("version = 2").unwrap();
        let err = CompiledRules::compile(file).unwrap_err();
        assert!(err.to_string().contains("version 2"), "err: {}", err);
    }

    #[test]
    fn test_compile_rejects_invalid_glob() {
        let file: RulesFile = toml::from_str(
            r#"
            [layers]
            domain = ["src/[oops"]
            "#,
        )
        .unwrap();
        let err = CompiledRules::compile(file).unwrap_err();
        assert!(err.to_string().contains("src/[oops"), "err: {}", err);
    }

    // ---------- validation ----------

    #[test]
    fn test_validate_ok() {
        let compiled = compile(FULL_RULES);
        let files = vec![
            "src/domain/order.ts".to_string(),
            "src/app/checkout.ts".to_string(),
            "src/infra/db.ts".to_string(),
            "src/unlayered.ts".to_string(),
        ];
        compiled.validate(&files).unwrap();
    }

    #[test]
    fn test_validate_unknown_layer() {
        let compiled = compile(
            r#"
            [layers]
            domain = ["src/domain/**"]

            [[rules.forbidden]]
            from = "domain"
            to = "infrastruture"
            "#,
        );
        let err = compiled.validate(&[]).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("unknown layer 'infrastruture'"),
            "err: {}",
            msg
        );
        assert!(msg.contains("domain"), "err: {}", msg);
    }

    #[test]
    fn test_validate_layer_overlap_names_file_and_layers() {
        let compiled = compile(
            r#"
            [layers]
            domain = ["src/domain/**"]
            everything = ["src/**"]
            "#,
        );
        let files = vec!["src/domain/order.ts".to_string()];
        let err = compiled.validate(&files).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("src/domain/order.ts"), "err: {}", msg);
        assert!(msg.contains("domain"), "err: {}", msg);
        assert!(msg.contains("everything"), "err: {}", msg);
    }

    #[test]
    fn test_validate_file_symbols_symbol_scope_is_error() {
        let compiled = compile(
            r#"
            [[rules.limit]]
            metric = "file_symbols"
            scope = "symbol"
            max = 50
            "#,
        );
        let err = compiled.validate(&[]).unwrap_err();
        assert!(err.to_string().contains("file_symbols"), "err: {}", err);
    }

    #[test]
    fn test_layer_of() {
        let compiled = compile(FULL_RULES);
        assert_eq!(compiled.layer_of("src/domain/order.ts"), Some("domain"));
        assert_eq!(compiled.layer_of("src/db/pool.ts"), Some("infrastructure"));
        assert_eq!(compiled.layer_of("src/other/x.ts"), None);
    }

    // ---------- evaluation ----------

    #[test]
    fn test_eval_forbidden() {
        let compiled = compile(FULL_RULES);
        let deps = vec![
            file_dep("src/domain/order.ts", "src/infra/db.ts"), // violation
            file_dep("src/app/checkout.ts", "src/infra/db.ts"), // fine
            file_dep("src/domain/order.ts", "src/domain/item.ts"), // fine
            symbol_dep("src/domain/pay.ts", "pay", "src/db/pool.ts", "query"), // violation
        ];
        let violations = eval_forbidden(&compiled, &deps);
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].rule, RuleKind::Forbidden);
        assert_eq!(violations[0].file, "src/domain/order.ts");
        assert_eq!(
            violations[0].reason,
            "Domain layer must stay persistence-agnostic"
        );
        assert_eq!(violations[1].file, "src/domain/pay.ts");
        assert_eq!(violations[1].line, Some(1));
    }

    #[test]
    fn test_eval_allowed_dependents_exempts_unlayered_and_self() {
        let compiled = compile(FULL_RULES);
        let deps = vec![
            file_dep("src/app/checkout.ts", "src/infra/db.ts"), // allowed
            file_dep("src/domain/order.ts", "src/infra/db.ts"), // violation
            file_dep("src/scripts/tool.ts", "src/infra/db.ts"), // unlayered -> exempt
            file_dep("src/infra/cache.ts", "src/infra/db.ts"),  // same layer -> exempt
        ];
        let violations = eval_allowed_dependents(&compiled, &deps);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule, RuleKind::AllowedDependents);
        assert_eq!(violations[0].file, "src/domain/order.ts");
        assert!(
            violations[0].reason.contains("domain"),
            "{}",
            violations[0].reason
        );
    }

    #[test]
    fn test_eval_limit_symbol_scope_with_exclude() {
        let compiled = compile(FULL_RULES); // fan_in <= 25 (symbol), exclude src/core/**
        let symbol_metrics = vec![
            metrics("hot", "src/app/hub.ts", 30, 0, 30), // violation
            metrics("ok", "src/app/x.ts", 25, 0, 25),    // at the limit: fine
            metrics("core", "src/core/bus.ts", 99, 0, 99), // excluded
        ];
        let violations = eval_limits(&compiled, &symbol_metrics, &[]);
        assert_eq!(violations.len(), 1);
        let v = &violations[0];
        assert_eq!(v.rule, RuleKind::Limit);
        assert_eq!(v.metric, Some(Metric::FanIn));
        assert_eq!(v.value, Some(30));
        assert_eq!(v.max, Some(25));
        assert_eq!(v.file, "src/app/hub.ts");
        match v.subject.as_ref().unwrap() {
            Endpoint::Symbol(s) => assert_eq!(s.name, "hot"),
            other => panic!("expected symbol subject, got {:?}", other),
        }
    }

    #[test]
    fn test_eval_limit_file_scope_sums_and_counts() {
        let compiled = compile(
            r#"
            [[rules.limit]]
            metric = "fan_out"
            scope = "file"
            max = 10

            [[rules.limit]]
            metric = "file_symbols"
            scope = "file"
            max = 2
            "#,
        );
        let symbol_metrics = vec![
            metrics("a", "src/big.ts", 0, 7, 14),
            metrics("b", "src/big.ts", 0, 6, 12), // sum fan_out = 13 > 10
            metrics("c", "src/small.ts", 0, 3, 6),
        ];
        let file_complexity = vec![
            FileComplexity {
                file_path: "src/big.ts".to_string(),
                complexity: 26,
                fan_out: 13,
                symbol_count: 3, // > 2 -> violation
            },
            FileComplexity {
                file_path: "src/small.ts".to_string(),
                complexity: 6,
                fan_out: 3,
                symbol_count: 1,
            },
        ];
        let violations = eval_limits(&compiled, &symbol_metrics, &file_complexity);
        assert_eq!(violations.len(), 2);
        assert_eq!(violations[0].metric, Some(Metric::FanOut));
        assert_eq!(violations[0].value, Some(13));
        assert_eq!(violations[0].file, "src/big.ts");
        assert_eq!(violations[1].metric, Some(Metric::FileSymbols));
        assert_eq!(violations[1].value, Some(3));
    }

    #[test]
    fn test_eval_no_new_dependents_without_against_reports_all_inbound() {
        let compiled = compile(FULL_RULES);
        let deps = vec![
            file_dep("src/app/checkout.ts", "src/legacy/billing.ts"), // inbound
            file_dep("src/legacy/a.ts", "src/legacy/billing.ts"),     // internal: fine
            file_dep("src/app/checkout.ts", "src/app/cart.ts"),       // unrelated
        ];
        let violations = eval_no_new_dependents(&compiled, &deps, None);
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].rule, RuleKind::NoNewDependents);
        assert_eq!(violations[0].file, "src/app/checkout.ts");
        assert_eq!(
            violations[0].reason,
            "Legacy module is frozen; do not add new callers"
        );
    }

    #[test]
    fn test_eval_no_new_dependents_with_against_requires_changed_source() {
        let compiled = compile(FULL_RULES);
        let deps = vec![
            file_dep("src/app/old.ts", "src/legacy/billing.ts"), // pre-existing
            file_dep("src/app/new.ts", "src/legacy/billing.ts"), // new dependent
        ];
        let changed: HashSet<String> = ["src/app/new.ts".to_string()].into();
        let violations = eval_no_new_dependents(&compiled, &deps, Some(&changed));
        assert_eq!(violations.len(), 1);
        assert_eq!(violations[0].file, "src/app/new.ts");
    }

    #[test]
    fn test_evaluate_against_filters_by_endpoint() {
        let compiled = compile(FULL_RULES);
        let deps = vec![
            file_dep("src/domain/old.ts", "src/infra/db.ts"), // untouched
            file_dep("src/domain/new.ts", "src/infra/db.ts"), // changed source
        ];
        let symbol_metrics = vec![
            metrics("hot", "src/app/hub.ts", 30, 0, 30), // untouched
            metrics("warm", "src/app/new2.ts", 40, 0, 40), // changed file
        ];

        // Without --against: everything is reported.
        let all = evaluate(&compiled, &deps, &symbol_metrics, &[], None);
        // 2 forbidden + 2 allowed_dependents + 2 limit = 6
        assert_eq!(all.len(), 6);

        let changed: HashSet<String> = [
            "src/domain/new.ts".to_string(),
            "src/app/new2.ts".to_string(),
        ]
        .into();
        let filtered = evaluate(&compiled, &deps, &symbol_metrics, &[], Some(&changed));
        assert_eq!(filtered.len(), 3);
        assert!(filtered
            .iter()
            .all(|v| v.endpoint_files().iter().any(|f| changed.contains(*f))));
    }

    #[test]
    fn test_violation_json_shape() {
        let compiled = compile(FULL_RULES);
        let deps = vec![symbol_dep(
            "src/domain/pay.ts",
            "pay",
            "src/infra/db.ts",
            "query",
        )];
        let violations = eval_forbidden(&compiled, &deps);
        let value = serde_json::to_value(&violations[0]).unwrap();
        assert_eq!(value["rule"], "forbidden");
        assert_eq!(
            value["reason"],
            "Domain layer must stay persistence-agnostic"
        );
        // Symbol endpoints serialize as full SymbolRefs.
        assert_eq!(value["from"]["name"], "pay");
        assert_eq!(value["from"]["file"], "src/domain/pay.ts");
        assert_eq!(value["to"]["kind"], "function");
        // Absent fields are omitted, not null.
        assert!(value.get("subject").is_none());
        assert!(value.get("metric").is_none());

        // File endpoints serialize as {"file": ...} objects.
        let file_violations = eval_forbidden(
            &compiled,
            &[file_dep("src/domain/order.ts", "src/infra/db.ts")],
        );
        let value = serde_json::to_value(&file_violations[0]).unwrap();
        assert_eq!(
            value["from"],
            serde_json::json!({"file": "src/domain/order.ts"})
        );
    }
}
