//! Synthetic data generator (RFC-117).
//!
//! Builds test rows for a contract using only the contract's own rules:
//! field types, `required`, `enum`, `min`/`max`, `min_length`/`max_length`,
//! `pattern`, `format`, date and timestamp types, quality rules
//! (completeness, freshness, uniqueness), field-bound metrics and
//! `compliance_mode`. It never reads stored events, quarantine rows or any
//! other real sample.
//!
//! Two modes:
//!   - `valid`: every row passes [`validate`] and the batch has no
//!     uniqueness duplicates.
//!   - `invalid`: every row breaks exactly one rule and carries a
//!     `broken_rule` label naming it.
//!
//! The validator is the oracle. Every candidate value and every finished row
//! is run through the same engine ingest uses, so a row is only emitted when
//! the validator agrees with its label. When a rule cannot be satisfied
//! (for example `min_length: 5` with `max_length: 3`), generation stops with
//! a [`SynthError`] naming the rule instead of emitting a bad row.
//!
//! Output is deterministic for a given seed, contract and engine version.
//! The one input from the outside world is `now_unix`, used only for
//! `freshness` rules, which are defined relative to the current time.

use crate::contract::{
    CompareOp, Contract, CrossFieldRule, FieldDefinition, FieldType, QualityRuleType,
};
use crate::validation::{
    compare_holds, concrete_rule, expand_cross_rule, pattern_for, string_format_ok, validate_at,
    CompiledContract, Violation, ViolationKind,
};
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};
use regex_syntax::hir::{Class, Hir, HirKind};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

/// Largest number of rows one call may ask for.
pub const MAX_ROWS: usize = 10_000;

/// Longest string the generator will build for one value.
const MAX_STRING_LEN: usize = 4096;
/// Candidate values tried per field before giving up on a rule.
const FIELD_ATTEMPTS: usize = 64;
/// Attempts that try a name-based realistic string before generic ones.
const NAMED_ATTEMPTS: usize = 6;
/// Whole-row attempts before giving up (covers row-level rules).
const ROW_ATTEMPTS: usize = 64;
/// Attempts to break one rule in isolation before skipping it.
const BREAK_ATTEMPTS: usize = 16;

/// Which kind of rows to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SynthMode {
    #[default]
    Valid,
    Invalid,
}

impl SynthMode {
    pub fn as_str(self) -> &'static str {
        match self {
            SynthMode::Valid => "valid",
            SynthMode::Invalid => "invalid",
        }
    }
}

/// Inputs for one generation run.
#[derive(Debug, Clone)]
pub struct SynthOptions {
    pub rows: usize,
    pub mode: SynthMode,
    pub seed: u64,
    /// Wall clock in Unix seconds. Only `freshness` rules read it.
    pub now_unix: i64,
}

/// A rule that could not be satisfied (or a request that cannot be served).
#[derive(Debug, Clone, PartialEq, Serialize, thiserror::Error)]
#[error("{message}")]
pub struct SynthError {
    /// Rule name, e.g. `max_length`, `uniqueness`, `enum`, `rows`.
    pub rule: String,
    /// Field path the rule sits on, when there is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    pub message: String,
}

impl SynthError {
    fn new(rule: &str, field: Option<&str>, message: impl Into<String>) -> Self {
        SynthError {
            rule: rule.to_string(),
            field: field.map(str::to_string),
            message: message.into(),
        }
    }
}

/// Label carried by every invalid row.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BrokenRule {
    /// Field path (or the undeclared key for `compliance_mode`).
    pub field: String,
    /// Rule name: `required`, `type`, `enum`, `min`, `max`, `min_length`,
    /// `max_length`, `pattern`, `format`, `completeness`, `freshness`,
    /// `uniqueness`, `metric_min`, `metric_max`, `cross_field` or
    /// `compliance_mode`. Array items use paths like `lines[0].sku`.
    pub rule: String,
    /// Metric name for `metric_min` / `metric_max`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
    /// For `cross_field`: the rule, e.g. `end_date >= start_date`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraint: Option<String>,
    /// The violation kind the validator reports for this row.
    pub violation_kind: ViolationKind,
    /// The validator's message for this row.
    pub message: String,
}

/// One generated invalid row and its label.
#[derive(Debug, Clone, Serialize)]
pub struct InvalidRow {
    pub record: Value,
    pub broken_rule: BrokenRule,
}

/// A rule invalid mode could not break on its own.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SkippedRule {
    pub field: String,
    pub rule: String,
    pub reason: String,
}

/// Result of a generation run.
#[derive(Debug, Clone, Serialize)]
pub struct SynthOutput {
    pub mode: SynthMode,
    pub seed: u64,
    /// `valid` mode only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rows: Vec<Value>,
    /// `invalid` mode only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub invalid_rows: Vec<InvalidRow>,
    /// Invalid mode: rules that could not be broken without also breaking
    /// another rule, so no row targets them.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub skipped_rules: Vec<SkippedRule>,
}

/// Generate rows for `compiled` according to `opts`.
pub fn generate(
    compiled: &CompiledContract,
    opts: &SynthOptions,
) -> Result<SynthOutput, SynthError> {
    if opts.rows == 0 || opts.rows > MAX_ROWS {
        return Err(SynthError::new(
            "rows",
            None,
            format!("rows must be between 1 and {MAX_ROWS}, got {}", opts.rows),
        ));
    }
    let mut g = Generator::new(compiled, opts)?;
    match opts.mode {
        SynthMode::Valid => {
            let mut rows = Vec::new();
            for _ in 0..opts.rows {
                let row = g.valid_row(false)?;
                g.remember(&row);
                rows.push(row);
            }
            // Belt and braces: the batch check ingest runs must agree.
            let dupes =
                crate::validation::check_uniqueness_batch(&compiled.contract.quality, &rows);
            if let Some((_, v)) = dupes.first() {
                return Err(SynthError::new(
                    "uniqueness",
                    Some(&v.field),
                    format!("internal check failed: {}", v.message),
                ));
            }
            Ok(SynthOutput {
                mode: opts.mode,
                seed: opts.seed,
                rows,
                invalid_rows: Vec::new(),
                skipped_rules: Vec::new(),
            })
        }
        SynthMode::Invalid => g.invalid_rows(opts.rows),
    }
}

/// Parse and compile contract YAML, mapping failures to a `contract` rule error.
pub fn compile_yaml(yaml: &str) -> Result<CompiledContract, SynthError> {
    let contract: Contract = crate::odcs::parse_contract_yaml(yaml)
        .map_err(|e| SynthError::new("contract", None, format!("invalid contract YAML: {e}")))?;
    CompiledContract::compile(contract)
        .map_err(|e| SynthError::new("contract", None, format!("contract does not compile: {e}")))
}

// ---------------------------------------------------------------------------
// Rule targets (invalid mode)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Target {
    Required(String),
    Type(String),
    Enum(String),
    Min(String),
    Max(String),
    MinLength(String),
    MaxLength(String),
    Pattern(String),
    Format(String),
    Completeness(String),
    Freshness(String, u64),
    Uniqueness(String),
    MetricMin(String, String, f64),
    MetricMax(String, String, f64),
    /// Index into `contract.cross_field`, and the rule's field path.
    CrossField(usize, String),
    Compliance,
}

impl Target {
    fn rule(&self) -> &'static str {
        match self {
            Target::Required(_) => "required",
            Target::Type(_) => "type",
            Target::Enum(_) => "enum",
            Target::Min(_) => "min",
            Target::Max(_) => "max",
            Target::MinLength(_) => "min_length",
            Target::MaxLength(_) => "max_length",
            Target::Pattern(_) => "pattern",
            Target::Format(_) => "format",
            Target::Completeness(_) => "completeness",
            Target::Freshness(..) => "freshness",
            Target::Uniqueness(_) => "uniqueness",
            Target::MetricMin(..) => "metric_min",
            Target::MetricMax(..) => "metric_max",
            Target::CrossField(..) => "cross_field",
            Target::Compliance => "compliance_mode",
        }
    }

    fn path(&self) -> &str {
        match self {
            Target::Required(p)
            | Target::Type(p)
            | Target::Enum(p)
            | Target::Min(p)
            | Target::Max(p)
            | Target::MinLength(p)
            | Target::MaxLength(p)
            | Target::Pattern(p)
            | Target::Format(p)
            | Target::Completeness(p)
            | Target::Freshness(p, _)
            | Target::Uniqueness(p)
            | Target::MetricMin(_, p, _)
            | Target::MetricMax(_, p, _)
            | Target::CrossField(_, p) => p,
            Target::Compliance => "<undeclared>",
        }
    }

    fn kind(&self) -> ViolationKind {
        match self {
            Target::Required(_) => ViolationKind::MissingRequiredField,
            Target::Type(_) => ViolationKind::TypeMismatch,
            Target::Enum(_) => ViolationKind::EnumViolation,
            Target::Min(_) | Target::Max(_) => ViolationKind::RangeViolation,
            Target::MinLength(_) | Target::MaxLength(_) => ViolationKind::LengthViolation,
            Target::Pattern(_) | Target::Format(_) => ViolationKind::PatternMismatch,
            Target::Completeness(_) => ViolationKind::CompletenessViolation,
            Target::Freshness(..) => ViolationKind::FreshnessViolation,
            Target::Uniqueness(_) => ViolationKind::UniquenessViolation,
            Target::MetricMin(..) | Target::MetricMax(..) => ViolationKind::MetricRangeViolation,
            Target::CrossField(..) => ViolationKind::CrossFieldViolation,
            Target::Compliance => ViolationKind::UndeclaredField,
        }
    }

    fn metric(&self) -> Option<String> {
        match self {
            Target::MetricMin(m, ..) | Target::MetricMax(m, ..) => Some(m.clone()),
            _ => None,
        }
    }

    /// Does `v` (the only violation on the row) name exactly this rule?
    fn matches(&self, v: &Violation) -> bool {
        if v.kind != self.kind() {
            return false;
        }
        let pattern_msg = v.message.contains("does not match required pattern");
        match self {
            Target::Compliance => true,
            Target::Pattern(p) => v.field == *p && pattern_msg,
            Target::Format(p) => v.field == *p && !pattern_msg,
            Target::Min(p) => v.field == *p && v.message.contains("below minimum"),
            Target::Max(p) => v.field == *p && v.message.contains("exceeds maximum"),
            Target::MinLength(p) => v.field == *p && v.message.contains("below minimum"),
            Target::MaxLength(p) => v.field == *p && v.message.contains("exceeds maximum"),
            Target::MetricMin(m, p, _) => {
                v.field == *p
                    && v.message.contains(&format!("'{m}'"))
                    && v.message.contains("below")
            }
            Target::MetricMax(m, p, _) => {
                v.field == *p
                    && v.message.contains(&format!("'{m}'"))
                    && v.message.contains("exceeds")
            }
            _ => v.field == self.path(),
        }
    }
}

/// Rule name the validator's violation points at (for error messages).
fn rule_for_violation(v: &Violation) -> &'static str {
    match v.kind {
        ViolationKind::MissingRequiredField if v.message.starts_with("Metric") => "metric",
        ViolationKind::MissingRequiredField => "required",
        ViolationKind::TypeMismatch => "type",
        ViolationKind::PatternMismatch if v.message.contains("does not match required pattern") => {
            "pattern"
        }
        ViolationKind::PatternMismatch => "format",
        ViolationKind::EnumViolation => "enum",
        ViolationKind::RangeViolation if v.message.contains("below minimum") => "min",
        ViolationKind::RangeViolation => "max",
        ViolationKind::LengthViolation if v.message.contains("below minimum") => "min_length",
        ViolationKind::LengthViolation => "max_length",
        ViolationKind::MetricRangeViolation if v.message.contains("below") => "metric_min",
        ViolationKind::MetricRangeViolation => "metric_max",
        ViolationKind::CompletenessViolation => "completeness",
        ViolationKind::FreshnessViolation => "freshness",
        ViolationKind::UniquenessViolation => "uniqueness",
        ViolationKind::UndeclaredField => "compliance_mode",
        ViolationKind::CrossFieldViolation => "cross_field",
        _ => "contract",
    }
}

fn unsatisfiable(v: &Violation) -> SynthError {
    let rule = rule_for_violation(v);
    SynthError::new(
        rule,
        Some(&v.field),
        format!(
            "cannot generate data that satisfies the {rule} rule on '{}' together with the contract's other rules: {}",
            v.field, v.message
        ),
    )
}

// ---------------------------------------------------------------------------
// Generator
// ---------------------------------------------------------------------------

struct Generator<'a> {
    compiled: &'a CompiledContract,
    rng: StdRng,
    seed: u64,
    now: i64,
    /// Ontology with row-level rules folded in (metric bounds, forced
    /// presence, freshness windows). Only used to build candidates; the
    /// real contract still judges every row.
    eff: Vec<FieldDefinition>,
    /// Single-field contracts used to check candidate values.
    oracles: HashMap<String, CompiledContract>,
    /// Parsed patterns, by regex source.
    hirs: HashMap<String, Hir>,
    /// Uniqueness rule paths and the values already emitted for each.
    unique: Vec<(String, HashSet<String>)>,
    /// Freshness windows (Unix seconds) for string and timestamp fields,
    /// which get RFC 3339 values inside the window.
    fresh: HashMap<String, (i64, i64)>,
}

impl<'a> Generator<'a> {
    fn new(compiled: &'a CompiledContract, opts: &SynthOptions) -> Result<Self, SynthError> {
        let contract = &compiled.contract;
        precheck_fields(&contract.ontology.entities, "")?;

        let mut eff = contract.ontology.entities.clone();
        let mut fresh = HashMap::new();
        for m in &contract.metrics {
            let Some(path) = &m.field else { continue };
            if m.min.is_none() && m.max.is_none() {
                continue;
            }
            if let (Some(lo), Some(hi)) = (m.min, m.max) {
                if lo > hi {
                    return Err(SynthError::new(
                        "metric",
                        Some(path),
                        format!("metric '{}' has min {lo} above max {hi}", m.name),
                    ));
                }
            }
            let template = numeric_template(path, m.min, m.max);
            let def = ensure_path(&mut eff, path, template, contract.compliance_mode, "metric")?;
            def.min = max_opt(def.min, m.min);
            def.max = min_opt(def.max, m.max);
        }
        for q in &contract.quality {
            match q.rule_type {
                QualityRuleType::Completeness => {
                    let mut t = string_template(&q.field);
                    t.min_length = Some(1);
                    let def = ensure_path(
                        &mut eff,
                        &q.field,
                        t,
                        contract.compliance_mode,
                        "completeness",
                    )?;
                    if is_stringish(&def.field_type) {
                        def.min_length = Some(def.min_length.unwrap_or(0).max(1));
                    }
                }
                QualityRuleType::Freshness => {
                    let Some(max_age) = q.max_age_seconds else {
                        continue;
                    };
                    let lo = (opts.now_unix - (max_age / 2) as i64) as f64;
                    let hi = opts.now_unix as f64;
                    let t = numeric_template(&q.field, Some(lo), Some(hi));
                    let def =
                        ensure_path(&mut eff, &q.field, t, contract.compliance_mode, "freshness")?;
                    if matches!(def.field_type, FieldType::String | FieldType::Timestamp) {
                        // RFC-118: RFC 3339 values inside the window.
                        fresh.insert(q.field.clone(), (lo as i64, hi as i64));
                        continue;
                    }
                    if matches!(def.field_type, FieldType::Float | FieldType::Any) {
                        def.field_type = FieldType::Integer;
                    }
                    def.min = max_opt(def.min, Some(lo));
                    def.max = min_opt(def.max, Some(hi));
                }
                _ => {}
            }
        }

        let unique = contract
            .quality
            .iter()
            .filter(|q| q.rule_type == QualityRuleType::Uniqueness)
            .map(|q| (q.field.clone(), HashSet::new()))
            .collect();

        Ok(Generator {
            compiled,
            rng: StdRng::seed_from_u64(opts.seed),
            seed: opts.seed,
            now: opts.now_unix,
            eff,
            oracles: HashMap::new(),
            hirs: HashMap::new(),
            unique,
            fresh,
        })
    }

    // -- rows ---------------------------------------------------------------

    /// A row that passes the real validator and repeats no unique value.
    /// `full` includes every optional field (invalid mode wants every rule
    /// reachable on the base row).
    fn valid_row(&mut self, full: bool) -> Result<Value, SynthError> {
        let mut last: Option<Violation> = None;
        let fields = self.eff.clone();
        for _ in 0..ROW_ATTEMPTS {
            let mut row = self.gen_object(&fields, "", full)?;
            for _ in 0..3 {
                self.repair_cross(&mut row);
            }
            let violations = self.row_violations(&row);
            match violations.into_iter().next() {
                None => return Ok(row),
                Some(v) => last = Some(v),
            }
        }
        Err(unsatisfiable(&last.expect("at least one attempt")))
    }

    /// Validator violations plus batch-uniqueness collisions against the
    /// rows already emitted.
    fn row_violations(&self, row: &Value) -> Vec<Violation> {
        let mut out = validate_at(self.compiled, row, self.now).violations;
        for (path, seen) in &self.unique {
            if let Some(v) = get_path(row, path) {
                if !v.is_null() && seen.contains(&v.to_string()) {
                    out.push(Violation {
                        field: path.clone(),
                        message: format!("duplicate value for unique field '{path}'"),
                        kind: ViolationKind::UniquenessViolation,
                        ..Default::default()
                    });
                }
            }
        }
        out
    }

    fn remember(&mut self, row: &Value) {
        for (path, seen) in &mut self.unique {
            if let Some(v) = get_path(row, path) {
                if !v.is_null() {
                    seen.insert(v.to_string());
                }
            }
        }
    }

    fn invalid_rows(&mut self, n: usize) -> Result<SynthOutput, SynthError> {
        let mut targets = self.targets();
        if targets.is_empty() {
            return Err(SynthError::new(
                "mode",
                None,
                "invalid mode needs at least one rule to break, and this contract declares none",
            ));
        }
        targets.shuffle(&mut self.rng);
        let mut dead = vec![false; targets.len()];
        let mut skipped = Vec::new();
        let mut out: Vec<InvalidRow> = Vec::new();

        for i in 0..n {
            let mut produced = None;
            for k in 0..targets.len() {
                let idx = (i + k) % targets.len();
                if dead[idx] {
                    continue;
                }
                let t = targets[idx].clone();
                if matches!(t, Target::Uniqueness(_)) && out.is_empty() {
                    continue; // the first row has nothing to duplicate
                }
                match self.break_rule(&t, &out)? {
                    Ok(row) => {
                        produced = Some(row);
                        break;
                    }
                    Err(reason) => {
                        dead[idx] = true;
                        skipped.push(SkippedRule {
                            field: t.path().to_string(),
                            rule: t.rule().to_string(),
                            reason,
                        });
                    }
                }
            }
            match produced {
                Some(row) => {
                    self.remember(&row.record);
                    out.push(row);
                }
                None => {
                    return Err(SynthError::new(
                        "mode",
                        None,
                        "no rule in this contract can be broken on its own, so no invalid rows can be generated",
                    ))
                }
            }
        }

        Ok(SynthOutput {
            mode: SynthMode::Invalid,
            seed: self.seed,
            rows: Vec::new(),
            invalid_rows: out,
            skipped_rules: skipped,
        })
    }

    fn break_rule(
        &mut self,
        t: &Target,
        prior: &[InvalidRow],
    ) -> Result<Result<InvalidRow, String>, SynthError> {
        if let Some(reason) = self.impossible_reason(t) {
            return Ok(Err(reason));
        }
        let mut seen: Vec<(String, usize)> = Vec::new();
        for _ in 0..BREAK_ATTEMPTS {
            let base = self.valid_row(true)?;
            let Some(mut mutated) = self.mutate(base, t, prior) else {
                continue;
            };
            self.repair_others(&mut mutated, t);
            let violations = self.row_violations(&mutated);
            for v in violations.iter().filter(|v| !t.matches(v)) {
                let what = match (&v.kind, &v.expected) {
                    (ViolationKind::CrossFieldViolation, Some(l)) => {
                        format!("the cross_field rule '{l}'")
                    }
                    _ => format!("the {} rule on '{}'", rule_for_violation(v), v.field),
                };
                match seen.iter_mut().find(|(w, _)| *w == what) {
                    Some((_, n)) => *n += 1,
                    None => seen.push((what, 1)),
                }
            }
            let constraint = match t {
                Target::CrossField(i, _) => Some(self.compiled.contract.cross_field[*i].label()),
                _ => None,
            };
            // Several cross-field rules can share a field; the label must
            // name the rule this row was built to break.
            let same_cross_rule = constraint.as_ref().is_none_or(|c| {
                violations.first().and_then(|v| v.expected.as_deref()) == Some(c.as_str())
            });
            if violations.len() == 1 && t.matches(&violations[0]) && same_cross_rule {
                let v = &violations[0];
                return Ok(Ok(InvalidRow {
                    record: mutated,
                    broken_rule: BrokenRule {
                        field: v.field.clone(),
                        rule: t.rule().to_string(),
                        metric: t.metric(),
                        constraint,
                        violation_kind: v.kind.clone(),
                        message: v.message.clone(),
                    },
                }));
            }
        }
        seen.sort_by_key(|s| std::cmp::Reverse(s.1));
        Ok(Err(match seen.first() {
            Some((what, _)) => format!(
                "every attempt to break the {} rule on '{}' also broke {what}",
                t.rule(),
                t.path()
            ),
            None => format!(
                "the generator found no value that breaks only the {} rule on '{}'",
                t.rule(),
                t.path()
            ),
        }))
    }

    /// Rules that cannot fail alone by the contract's own logic, with the
    /// reason. Checked before trying, so the explanation is exact.
    fn impossible_reason(&self, t: &Target) -> Option<String> {
        let c = &self.compiled.contract;
        let def = find_def(&c.ontology.entities, t.path());
        let completeness = |p: &str| {
            c.quality
                .iter()
                .any(|q| q.rule_type == QualityRuleType::Completeness && q.field == p)
        };
        let metric_reads = |p: &str| {
            c.metrics
                .iter()
                .find(|m| m.field.as_deref() == Some(p) && (m.min.is_some() || m.max.is_some()))
        };
        match t {
            Target::Required(p) => {
                if completeness(p) {
                    return Some(format!(
                        "a missing '{p}' always also fails the completeness rule on it"
                    ));
                }
                if let Some(m) = metric_reads(p) {
                    return Some(format!(
                        "metric '{}' reads '{p}', so a missing value always also fails the metric",
                        m.name
                    ));
                }
            }
            Target::Completeness(p) => {
                let d = def?;
                let strict_string = d.min_length.is_some_and(|m| m >= 1)
                    || d.pattern.is_some()
                    || d.allowed_values.is_some()
                    || d.format
                        .as_deref()
                        .is_some_and(|f| !string_format_ok(f, ""))
                    || !matches!(d.field_type, FieldType::String | FieldType::Any);
                if d.required && !c.null_as_absent && strict_string {
                    return Some(format!(
                        "'{p}' is required and an empty string fails its own rules, so a missing, null or empty value always also breaks required, type or format"
                    ));
                }
            }
            Target::Format(p) => {
                let d = def?;
                let fresh = c
                    .quality
                    .iter()
                    .any(|q| q.rule_type == QualityRuleType::Freshness && q.field == *p);
                if fresh && matches!(d.field_type, FieldType::Timestamp) {
                    return Some(format!(
                        "the freshness rule reads '{p}' as an RFC 3339 date-time, so any value that fails the timestamp format also fails freshness"
                    ));
                }
            }
            Target::Type(p) => {
                let d = def?;
                if let Some(m) = metric_reads(p) {
                    if !matches!(d.field_type, FieldType::Integer) {
                        return Some(format!(
                            "metric '{}' reads '{p}' as a number, so any non-number also fails the metric",
                            m.name
                        ));
                    }
                }
            }
            Target::Max(p) | Target::Min(p) => {
                let d = def?;
                let is_max = matches!(t, Target::Max(_));
                for m in c
                    .metrics
                    .iter()
                    .filter(|m| m.field.as_deref() == Some(p.as_str()))
                {
                    if is_max {
                        if let (Some(fm), Some(mm)) = (d.max, m.max) {
                            if mm <= fm {
                                return Some(format!(
                                    "any '{p}' above its max {fm} is also above metric '{}' max {mm}",
                                    m.name
                                ));
                            }
                        }
                    } else if let (Some(fm), Some(mm)) = (d.min, m.min) {
                        if mm >= fm {
                            return Some(format!(
                                "any '{p}' below its min {fm} is also below metric '{}' min {mm}",
                                m.name
                            ));
                        }
                    }
                }
                // Cross-field comparisons whose other side cannot follow.
                for rule in &c.cross_field {
                    let (Some(op), Some(other)) = (rule.op, &rule.other) else {
                        continue;
                    };
                    // Normalise to big >= small.
                    let (big, small) = match op {
                        CompareOp::Gt | CompareOp::Gte => (&rule.field, other),
                        CompareOp::Lt | CompareOp::Lte => (other, &rule.field),
                        _ => continue,
                    };
                    let (big, small) = (big.replace("[]", "[0]"), small.replace("[]", "[0]"));
                    let (mine, theirs) = if *p == big && !is_max {
                        (d.min, find_def(&c.ontology.entities, &small))
                    } else if *p == small && is_max {
                        (d.max, find_def(&c.ontology.entities, &big))
                    } else {
                        continue;
                    };
                    let (Some(bound), Some(o)) = (mine, theirs) else {
                        continue;
                    };
                    if !o.required {
                        continue; // the other side can be left out
                    }
                    let blocked = if is_max {
                        o.max.is_some_and(|om| om <= bound)
                    } else {
                        o.min.is_some_and(|om| om >= bound)
                    };
                    if blocked {
                        let (side, ob) = if is_max {
                            ("above its max", o.max.unwrap_or_default())
                        } else {
                            ("below its min", o.min.unwrap_or_default())
                        };
                        let rel = if is_max { "max" } else { "min" };
                        return Some(format!(
                            "any '{p}' {side} {bound} also fails the cross_field rule '{}', because '{}' has {rel} {ob} and is required",
                            rule.label(),
                            if *p == big { &small } else { &big }
                        ));
                    }
                }
            }
            _ => {}
        }
        None
    }

    /// After `mutate`, fix every other rule the change knocked over while
    /// leaving the target field alone (RFC-118): move the other side of a
    /// cross-field comparison, flip a `required_if` condition, or draw a
    /// new value for an unrelated field.
    fn repair_others(&mut self, row: &mut Value, t: &Target) {
        let tp = t.path().to_string();
        for _ in 0..4 {
            let others: Vec<Violation> = self
                .row_violations(row)
                .into_iter()
                .filter(|v| !t.matches(v))
                .collect();
            if others.is_empty() {
                return;
            }
            for v in others {
                if v.kind == ViolationKind::CrossFieldViolation {
                    if let Some(rule) = self.concrete_for(&v, row) {
                        self.fix_cross_holding(row, &rule, &tp);
                    }
                } else if !touches(&v.field, &tp) {
                    if let Some(nv) = self.regen(&v.field) {
                        set_path(row, &v.field, nv);
                    }
                }
            }
        }
    }

    /// The concrete cross-field rule behind violation `v` on `row`.
    fn concrete_for(&self, v: &Violation, row: &Value) -> Option<CrossFieldRule> {
        let label = v.expected.as_deref()?;
        self.compiled
            .contract
            .cross_field
            .iter()
            .filter(|r| r.label() == label)
            .flat_map(|r| expand_cross_rule(r, row))
            .find(|r| r.field == v.field)
    }

    fn fix_cross_holding(&mut self, row: &mut Value, rule: &CrossFieldRule, tp: &str) {
        if let (Some(op), Some(other)) = (rule.op, &rule.other) {
            let field_fixed = touches(&rule.field, tp);
            let other_fixed = touches(other, tp);
            if field_fixed && other_fixed {
                return;
            }
            let (move_path, keep_path) = if field_fixed {
                (other.clone(), rule.field.clone())
            } else {
                (rule.field.clone(), other.clone())
            };
            let Some(keep) = get_path(row, &keep_path).cloned() else {
                return;
            };
            let holds = |cand: &Value| {
                if move_path == rule.field {
                    compare_holds(op, cand, &keep)
                } else {
                    compare_holds(op, &keep, cand)
                }
            };
            if op == CompareOp::Eq {
                set_path(row, &move_path, keep);
                return;
            }
            for _ in 0..24 {
                if let Some(cand) = self.regen(&move_path) {
                    if holds(&cand) {
                        set_path(row, &move_path, cand);
                        return;
                    }
                }
            }
            // Last resort: leave an optional other side out, which skips
            // the comparison.
            let optional = find_def(&self.compiled.contract.ontology.entities, &move_path)
                .is_some_and(|d| !d.required);
            if optional {
                remove_path(row, &move_path);
            }
        } else if let Some(cond) = &rule.required_if {
            if touches(&rule.field, tp) {
                if touches(&cond.field, tp) {
                    return;
                }
                for _ in 0..16 {
                    if let Some(cand) = self.regen(&cond.field) {
                        if cand != cond.equals {
                            set_path(row, &cond.field, cand);
                            return;
                        }
                    }
                }
            } else if let Some(nv) = self.regen(&rule.field) {
                set_path(row, &rule.field, nv);
            }
        }
    }

    /// Every rule invalid mode can aim at, in contract order.
    fn targets(&self) -> Vec<Target> {
        let c = &self.compiled.contract;
        let mut out = Vec::new();
        collect_field_targets(&c.ontology.entities, "", false, &mut out);
        for q in &c.quality {
            match q.rule_type {
                QualityRuleType::Completeness => out.push(Target::Completeness(q.field.clone())),
                QualityRuleType::Freshness => {
                    if let Some(a) = q.max_age_seconds {
                        out.push(Target::Freshness(q.field.clone(), a));
                    }
                }
                QualityRuleType::Uniqueness => out.push(Target::Uniqueness(q.field.clone())),
                QualityRuleType::Validity => {}
            }
        }
        for m in &c.metrics {
            let Some(p) = &m.field else { continue };
            if let Some(lo) = m.min {
                out.push(Target::MetricMin(m.name.clone(), p.clone(), lo));
            }
            if let Some(hi) = m.max {
                out.push(Target::MetricMax(m.name.clone(), p.clone(), hi));
            }
        }
        for (i, r) in c.cross_field.iter().enumerate() {
            out.push(Target::CrossField(i, r.field.replacen("[]", "[0]", 1)));
        }
        if c.compliance_mode {
            out.push(Target::Compliance);
        }
        out
    }

    /// Change `row` so it (should) break only `t`. `None` = not applicable.
    fn mutate(&mut self, mut row: Value, t: &Target, prior: &[InvalidRow]) -> Option<Value> {
        let def = find_def(&self.compiled.contract.ontology.entities, t.path()).cloned();
        let new_value: Option<Value> = match t {
            Target::CrossField(i, _) => {
                let rule = &self.compiled.contract.cross_field[*i];
                let rule = if rule.wildcard_array().is_some() {
                    concrete_rule(rule, 0)
                } else {
                    rule.clone()
                };
                return self.break_cross(row, &rule);
            }
            Target::Required(p) => {
                remove_path(&mut row, p);
                return Some(row);
            }
            Target::Compliance => {
                let key = format!("undeclared_field_{}", self.rng.gen_range(0..1_000_000));
                row.as_object_mut()?.insert(key, json!("synthetic"));
                return Some(row);
            }
            Target::Completeness(p) => match self.rng.gen_range(0..3) {
                0 => {
                    remove_path(&mut row, p);
                    return Some(row);
                }
                1 => Some(Value::Null),
                _ => Some(json!("")),
            },
            Target::Type(p) => {
                let d = def?;
                let n = self.rng.gen_range(1..100_000);
                let fresh_age = self.compiled.contract.quality.iter().find_map(|q| {
                    (q.rule_type == QualityRuleType::Freshness && q.field == *p)
                        .then_some(q.max_age_seconds)
                        .flatten()
                });
                if let Some(age) = fresh_age {
                    // Keep the value fresh so only the type rule fails.
                    let secs = self.now - self.rng.gen_range(0..=(age / 2) as i64);
                    return set_path(
                        &mut row,
                        p,
                        match d.field_type {
                            FieldType::Integer | FieldType::Float => json!(rfc3339(secs)),
                            _ => json!(secs),
                        },
                    )
                    .map(|_| row);
                }
                if d.field_type == FieldType::Integer && d.allowed_values.is_none() {
                    // A fractional number inside the bounds fails only `type`
                    // (metrics and ranges still see a number in range).
                    let metric = self
                        .compiled
                        .contract
                        .metrics
                        .iter()
                        .filter(|m| m.field.as_deref() == Some(p.as_str()));
                    let mut lo = d.min.unwrap_or(f64::MIN);
                    let mut hi = d.max.unwrap_or(f64::MAX);
                    for m in metric {
                        lo = lo.max(m.min.unwrap_or(f64::MIN));
                        hi = hi.min(m.max.unwrap_or(f64::MAX));
                    }
                    let base = if lo > f64::MIN {
                        lo.ceil()
                    } else if hi < f64::MAX {
                        hi.floor() - 1.0
                    } else {
                        n as f64
                    };
                    let v = base + 0.5;
                    if v >= lo && v <= hi {
                        return set_path(&mut row, p, json!(v)).map(|_| row);
                    }
                }
                Some(match d.field_type {
                    FieldType::String | FieldType::Date | FieldType::Timestamp => json!(n),
                    FieldType::Integer | FieldType::Float => json!(format!("not_a_number_{n}")),
                    FieldType::Boolean => json!(format!("yes_{n}")),
                    FieldType::Object => json!(format!("not_an_object_{n}")),
                    FieldType::Array => json!(format!("not_an_array_{n}")),
                    FieldType::Any => return None,
                })
            }
            Target::Enum(p) => {
                let d = def?;
                let mut relaxed = d.clone();
                relaxed.allowed_values = None;
                let allowed = d.allowed_values.clone().unwrap_or_default();
                let mut found = None;
                for _ in 0..8 {
                    let v = self.gen_checked(&relaxed, p, "enum").ok()?;
                    if !allowed.contains(&v) {
                        found = Some(v);
                        break;
                    }
                }
                found
            }
            Target::Min(_) => {
                let d = def?;
                let lo = d.min?;
                let step = self.rng.gen_range(1..4) as f64;
                Some(match d.field_type {
                    FieldType::Integer => json!(lo.ceil() as i64 - step as i64),
                    _ => json!(round2(lo - step - self.rng.gen::<f64>())),
                })
            }
            Target::Max(_) => {
                let d = def?;
                let hi = d.max?;
                let step = self.rng.gen_range(1..4) as f64;
                Some(match d.field_type {
                    FieldType::Integer => json!(hi.floor() as i64 + step as i64),
                    _ => json!(round2(hi + step + self.rng.gen::<f64>())),
                })
            }
            Target::MinLength(p) => {
                let d = def?;
                let ml = d.min_length?;
                let mut relaxed = as_plain_string(&d);
                relaxed.min_length = None;
                relaxed.max_length = Some(ml.checked_sub(1)?);
                self.gen_checked(&relaxed, p, "min_length").ok()
            }
            Target::MaxLength(p) => {
                let d = def?;
                let ml = d.max_length?;
                let mut relaxed = as_plain_string(&d);
                relaxed.min_length = Some(ml + 1);
                relaxed.max_length = Some(ml + 1 + self.rng.gen_range(0..4));
                self.gen_checked(&relaxed, p, "max_length").ok()
            }
            Target::Pattern(p) => {
                let d = def?;
                let re = pattern_for(&self.compiled.patterns, p)?.clone();
                let mut relaxed = d.clone();
                relaxed.allowed_values = None;
                relaxed.pattern = None;
                // No name-based guess: it would aim for a well-formed value.
                relaxed.name = String::new();
                let mut found = None;
                for _ in 0..8 {
                    let v = self.gen_checked(&relaxed, p, "pattern").ok()?;
                    if v.as_str().is_some_and(|s| !re.is_match(s)) {
                        found = Some(v);
                        break;
                    }
                }
                found
            }
            Target::Format(p) => {
                let d = def?;
                let mut relaxed = as_plain_string(&d);
                relaxed.format = None;
                relaxed.name = String::new();
                let mut found = None;
                for _ in 0..8 {
                    let v = self.gen_checked(&relaxed, p, "format").ok()?;
                    let s = v.as_str()?;
                    let bad = match d.field_type {
                        FieldType::Date => {
                            chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").is_err()
                        }
                        FieldType::Timestamp => chrono::DateTime::parse_from_rfc3339(s).is_err(),
                        _ => !string_format_ok(d.format.as_deref().unwrap_or(""), s),
                    };
                    if bad {
                        found = Some(v);
                        break;
                    }
                }
                found
            }
            Target::Freshness(p, max_age) => {
                let age = *max_age as i64 + 1 + self.rng.gen_range(0..86_400);
                if self.fresh.contains_key(p) {
                    Some(json!(rfc3339(self.now - age)))
                } else {
                    Some(json!(self.now - age))
                }
            }
            Target::Uniqueness(p) => {
                let candidates: Vec<&Value> = prior
                    .iter()
                    .filter_map(|r| get_path(&r.record, p))
                    .filter(|v| !v.is_null())
                    .collect();
                candidates.choose(&mut self.rng).map(|v| (*v).clone())
            }
            Target::MetricMin(_, _, lo) => {
                let step = self.rng.gen_range(1..4);
                Some(if is_integer_field(def.as_ref()) {
                    json!(lo.ceil() as i64 - step)
                } else {
                    json!(round2(lo - step as f64 - self.rng.gen::<f64>()))
                })
            }
            Target::MetricMax(_, _, hi) => {
                let step = self.rng.gen_range(1..4);
                Some(if is_integer_field(def.as_ref()) {
                    json!(hi.floor() as i64 + step)
                } else {
                    json!(round2(hi + step as f64 + self.rng.gen::<f64>()))
                })
            }
        };
        let v = new_value?;
        set_path(&mut row, t.path(), v)?;
        Some(row)
    }

    // -- cross-field rules ---------------------------------------------------

    /// Nudge a fresh row toward the contract's cross-field rules: swap two
    /// values that are out of order, copy for `eq`, regenerate for `ne`, and
    /// fill a conditionally required field. The validator still judges the
    /// result, and the row is regenerated if it fails.
    fn repair_cross(&mut self, row: &mut Value) {
        let rules: Vec<CrossFieldRule> = self
            .compiled
            .contract
            .cross_field
            .iter()
            .flat_map(|r| expand_cross_rule(r, row))
            .collect();
        for rule in &rules {
            if let (Some(op), Some(other)) = (rule.op, &rule.other) {
                let (Some(a), Some(b)) = (
                    get_path(row, &rule.field).filter(|v| !v.is_null()).cloned(),
                    get_path(row, other).filter(|v| !v.is_null()).cloned(),
                ) else {
                    continue;
                };
                if compare_holds(op, &a, &b) {
                    continue;
                }
                match op {
                    CompareOp::Eq => {
                        set_path(row, &rule.field, b);
                    }
                    CompareOp::Ne => {
                        if let Some(v) = self.regen(&rule.field) {
                            set_path(row, &rule.field, v);
                        }
                    }
                    _ => {
                        if compare_holds(op, &b, &a) {
                            set_path(row, &rule.field, b);
                            set_path(row, other, a);
                        } else if let Some(v) = self.regen(&rule.field) {
                            set_path(row, &rule.field, v);
                        }
                    }
                }
            } else if let Some(cond) = &rule.required_if {
                let holds = get_path(row, &cond.field) == Some(&cond.equals);
                let present = get_path(row, &rule.field).is_some_and(|v| !v.is_null());
                if holds && !present {
                    if let Some(v) = self.regen(&rule.field) {
                        set_path(row, &rule.field, v);
                    }
                }
            }
        }
    }

    /// A fresh valid value for the field at `path`, from the generation
    /// ontology. `None` when the path is not declared.
    fn regen(&mut self, path: &str) -> Option<Value> {
        let def = find_def(&self.eff, path)?.clone();
        self.gen_field(&def, path, true).ok()
    }

    /// Make `row` fail `rule` (and, ideally, nothing else).
    fn break_cross(&mut self, mut row: Value, rule: &CrossFieldRule) -> Option<Value> {
        if let (Some(op), Some(other)) = (rule.op, &rule.other) {
            let a = get_path(&row, &rule.field)?.clone();
            let b = get_path(&row, other)?.clone();
            if a.is_null() || b.is_null() {
                return None;
            }
            match op {
                CompareOp::Ne => {
                    set_path(&mut row, &rule.field, b);
                }
                _ => {
                    // Strategies, tried from a seeded random start: swap the
                    // pair, redraw `field`, redraw `other`, redraw both.
                    // Redrawn values stay inside each field's own rules, so
                    // whichever side has room to move is the one that moves.
                    let start = self.rng.gen_range(0..4usize);
                    let mut done = false;
                    for k in 0..4 {
                        if done {
                            break;
                        }
                        match (start + k) % 4 {
                            0 => {
                                if op != CompareOp::Eq && !compare_holds(op, &b, &a) {
                                    set_path(&mut row, &rule.field, b.clone());
                                    set_path(&mut row, other, a.clone());
                                    done = true;
                                }
                            }
                            1 => {
                                for _ in 0..8 {
                                    let Some(v) = self.regen(&rule.field) else {
                                        break;
                                    };
                                    if !compare_holds(op, &v, &b) {
                                        set_path(&mut row, &rule.field, v);
                                        done = true;
                                        break;
                                    }
                                }
                            }
                            2 => {
                                for _ in 0..8 {
                                    let Some(w) = self.regen(other) else { break };
                                    if !compare_holds(op, &a, &w) {
                                        set_path(&mut row, other, w);
                                        done = true;
                                        break;
                                    }
                                }
                            }
                            _ => {
                                for _ in 0..8 {
                                    let (Some(v), Some(w)) =
                                        (self.regen(&rule.field), self.regen(other))
                                    else {
                                        break;
                                    };
                                    if !compare_holds(op, &v, &w) {
                                        set_path(&mut row, &rule.field, v);
                                        set_path(&mut row, other, w);
                                        done = true;
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if !done {
                        return None;
                    }
                }
            }
            Some(row)
        } else if let Some(cond) = &rule.required_if {
            set_path(&mut row, &cond.field, cond.equals.clone())?;
            remove_path(&mut row, &rule.field);
            Some(row)
        } else {
            None
        }
    }

    // -- values -------------------------------------------------------------

    fn gen_object(
        &mut self,
        fields: &[FieldDefinition],
        prefix: &str,
        full: bool,
    ) -> Result<Value, SynthError> {
        let mut obj = Map::new();
        for f in fields {
            let path = join(prefix, &f.name);
            if !(f.required || full || self.rng.gen_bool(0.75)) {
                continue;
            }
            let v = self.gen_field(f, &path, full)?;
            obj.insert(f.name.clone(), v);
        }
        Ok(Value::Object(obj))
    }

    fn gen_field(
        &mut self,
        f: &FieldDefinition,
        path: &str,
        full: bool,
    ) -> Result<Value, SynthError> {
        match f.field_type {
            FieldType::Object if f.allowed_values.is_none() => {
                let props = f.properties.clone().unwrap_or_default();
                self.gen_object(&props, path, full)
            }
            FieldType::Array if f.allowed_values.is_none() => {
                let n = self.rng.gen_range(if full { 1 } else { 0 }..=3);
                let mut items = Vec::with_capacity(n);
                for i in 0..n {
                    let ip = format!("{path}[{i}]");
                    match &f.items {
                        Some(item) => {
                            let item = (**item).clone();
                            items.push(self.gen_field(&item, &ip, full)?)
                        }
                        None => items.push(json!(self.alnum(6))),
                    }
                }
                Ok(Value::Array(items))
            }
            _ => self.gen_checked(f, path, "field"),
        }
    }

    /// Generate a scalar candidate for `f` and keep it only when a one-field
    /// contract built from `f` accepts it.
    fn gen_checked(
        &mut self,
        f: &FieldDefinition,
        path: &str,
        why: &str,
    ) -> Result<Value, SynthError> {
        let key = format!("{why}:{path}:{}", oracle_key(f));
        if !self.oracles.contains_key(&key) {
            let mut c = self.compiled.contract.clone();
            let mut d = f.clone();
            d.name = "v".into();
            d.required = true;
            d.transform = None;
            c.ontology.entities = vec![d];
            c.metrics.clear();
            c.quality.clear();
            c.compliance_mode = false;
            c.envelope = None;
            let oracle = CompiledContract::compile(c).map_err(|e| {
                SynthError::new("pattern", Some(path), format!("field '{path}': {e}"))
            })?;
            self.oracles.insert(key.clone(), oracle);
        }
        let mut last = None;
        for attempt in 0..FIELD_ATTEMPTS {
            let candidate = self.raw_value(f, path, attempt)?;
            let wrapped = json!({ "v": candidate });
            let res = validate_at(&self.oracles[&key], &wrapped, self.now);
            if res.passed {
                return Ok(candidate);
            }
            last = res.violations.into_iter().next();
        }
        let mut v = last.expect("at least one attempt");
        v.field = path.to_string();
        v.message = v.message.replace("'v'", &format!("'{path}'"));
        Err(unsatisfiable(&v))
    }

    fn raw_value(
        &mut self,
        f: &FieldDefinition,
        path: &str,
        attempt: usize,
    ) -> Result<Value, SynthError> {
        if let Some(allowed) = &f.allowed_values {
            if allowed.is_empty() {
                return Err(SynthError::new(
                    "enum",
                    Some(path),
                    format!("field '{path}' has an empty enum, so no value can pass"),
                ));
            }
            return Ok(allowed
                .choose(&mut self.rng)
                .cloned()
                .unwrap_or(Value::Null));
        }
        if let Some(&(lo, hi)) = self.fresh.get(path) {
            if matches!(f.field_type, FieldType::String | FieldType::Timestamp) {
                let secs = self.rng.gen_range(lo..=hi.max(lo));
                return Ok(json!(rfc3339(secs)));
            }
        }
        Ok(match f.field_type {
            FieldType::Integer => {
                let (lo, hi) = int_bounds(f, path)?;
                json!(self.rng.gen_range(lo..=hi))
            }
            FieldType::Float => {
                let (lo, hi) = float_bounds(f, path)?;
                let raw = if lo == hi {
                    lo
                } else {
                    self.rng.gen_range(lo..=hi)
                };
                let rounded = (raw * 100.0).round() / 100.0;
                let v = if rounded >= lo && rounded <= hi {
                    rounded
                } else {
                    raw
                };
                json!(v)
            }
            FieldType::Boolean => json!(self.rng.gen_bool(0.5)),
            FieldType::Object => json!({}),
            FieldType::Array => json!([]),
            FieldType::String | FieldType::Date | FieldType::Timestamp | FieldType::Any => {
                json!(self.string_value(f, path, attempt)?)
            }
        })
    }

    fn string_value(
        &mut self,
        f: &FieldDefinition,
        path: &str,
        attempt: usize,
    ) -> Result<String, SynthError> {
        let semantic = match f.field_type {
            FieldType::Date => Some(self.date()),
            FieldType::Timestamp => Some(self.timestamp()),
            _ => match f.format.as_deref().map(str::to_ascii_lowercase).as_deref() {
                Some("email") => {
                    // "@example.com" is 12 characters; size the local part
                    // so the whole address fits the length rules.
                    let n = if f.min_length.is_some() || f.max_length.is_some() {
                        let (lo, hi) = self.len_range(f);
                        let n = if lo >= hi {
                            lo
                        } else {
                            self.rng.gen_range(lo..=hi)
                        };
                        n.saturating_sub(12).max(1)
                    } else {
                        self.rng.gen_range(3..10)
                    };
                    if f.min_length.is_none() && f.max_length.is_none() {
                        Some(self.realistic_email())
                    } else {
                        let local = self.lower(n);
                        let tld = ["com", "org", "net"][self.rng.gen_range(0..3)];
                        Some(format!("{local}@example.{tld}"))
                    }
                }
                Some("uuid") => {
                    let bytes: [u8; 16] = self.rng.gen();
                    Some(
                        uuid::Builder::from_random_bytes(bytes)
                            .into_uuid()
                            .to_string(),
                    )
                }
                Some("uri") | Some("url") => Some(format!("https://example.com/{}", self.lower(8))),
                Some("date") => Some(self.date()),
                Some("date-time") | Some("datetime") | Some("timestamp") => Some(self.timestamp()),
                _ => None,
            },
        };
        // No type or format shape: guess one from the field name (`email`,
        // `first_name`, `phone`, `country`, ...). Only the first few attempts
        // use it, and the oracle still checks every declared rule, so a guess
        // that breaks a length or pattern rule falls back to the generic value.
        let named = semantic.is_none()
            && f.format.is_none()
            && matches!(f.field_type, FieldType::String | FieldType::Any)
            && attempt
                < if f.pattern.is_some() {
                    1
                } else {
                    NAMED_ATTEMPTS
                };
        let semantic = match semantic {
            Some(s) => Some(s),
            None if named => self.named_string(&f.name),
            None => None,
        };
        // With a pattern, alternate between the type/format-shaped value
        // and one built from the regex; the oracle keeps whichever fits.
        if let Some(pat) = &f.pattern {
            if semantic.is_none() || (attempt % 2 == 1 && !named) {
                return self.gen_from_pattern(pat, path, f.min_length.unwrap_or(0));
            }
        }
        if let Some(s) = semantic {
            return Ok(s);
        }
        if f.min_length.unwrap_or(0) > MAX_STRING_LEN {
            return Err(SynthError::new(
                "min_length",
                Some(path),
                format!(
                    "field '{path}' min_length {} is above the generator limit of {MAX_STRING_LEN}",
                    f.min_length.unwrap_or(0)
                ),
            ));
        }
        let (lo, hi) = self.len_range(f);
        let len = if lo >= hi {
            lo
        } else {
            self.rng.gen_range(lo..=hi)
        };
        Ok(self.alnum(len))
    }

    /// Target length range for a free-form string: inside the contract's
    /// bounds, non-empty when allowed, and short when unbounded.
    fn len_range(&self, f: &FieldDefinition) -> (usize, usize) {
        let lo = f.min_length.unwrap_or(0);
        let hi = f.max_length.unwrap_or(lo.max(8) + 8).min(lo.max(16) + 16);
        let lo = if lo == 0 && hi > 0 { 1 } else { lo };
        (lo, hi)
    }

    fn gen_from_pattern(
        &mut self,
        pat: &str,
        path: &str,
        min_len: usize,
    ) -> Result<String, SynthError> {
        if !self.hirs.contains_key(pat) {
            let hir = regex_syntax::ParserBuilder::new()
                .build()
                .parse(pat)
                .map_err(|e| {
                    SynthError::new("pattern", Some(path), format!("field '{path}': {e}"))
                })?;
            self.hirs.insert(pat.to_string(), hir);
        }
        let hir = self.hirs[pat].clone();
        let mut out = String::new();
        // Unbounded repeats may need to run long to reach min_length.
        let extra = self
            .rng
            .gen_range(0..=(6 + min_len.min(MAX_STRING_LEN)) as u32);
        if !gen_hir(&hir, &mut self.rng, &mut out, extra) {
            return Err(SynthError::new(
                "pattern",
                Some(path),
                format!("field '{path}': pattern /{pat}/ cannot match any string the generator can build"),
            ));
        }
        Ok(out)
    }

    fn pick(&mut self, xs: &[&str]) -> String {
        xs[self.rng.gen_range(0..xs.len())].to_string()
    }

    fn realistic_email(&mut self) -> String {
        let first = self.pick(FIRST_NAMES).to_ascii_lowercase();
        let last = self.pick(LAST_NAMES).to_ascii_lowercase();
        let n = self.rng.gen_range(1..100);
        let domain = self.pick(&["example.com", "example.org", "example.net"]);
        match self.rng.gen_range(0..3) {
            0 => format!("{first}.{last}@{domain}"),
            1 => format!("{}{last}{n}@{domain}", &first[..1]),
            _ => format!("{first}{n}@{domain}"),
        }
    }

    /// A realistic value for a free-form string field, chosen from its name.
    /// `None` when the name gives no hint. Uses the seeded RNG, so output
    /// stays deterministic.
    fn named_string(&mut self, name: &str) -> Option<String> {
        let toks = name_tokens(name);
        let has = |w: &str| toks.iter().any(|t| t == w);
        let last_tok = toks.last().map(String::as_str).unwrap_or("");
        let v = if has("email") || has("mail") {
            self.realistic_email()
        } else if has("uuid") || has("guid") || last_tok == "id" {
            let bytes: [u8; 16] = self.rng.gen();
            uuid::Builder::from_random_bytes(bytes)
                .into_uuid()
                .to_string()
        } else if has("phone") || has("mobile") || has("tel") || has("telephone") || has("fax") {
            format!(
                "+1-{}-555-{:04}",
                self.rng.gen_range(201..990),
                self.rng.gen_range(100..200)
            )
        } else if has("url") || has("website") || has("homepage") || has("link") || has("uri") {
            let w = self.pick(WORDS);
            format!("https://www.example.com/{w}")
        } else if (has("first") && has("name"))
            || has("firstname")
            || has("given")
            || has("forename")
        {
            self.pick(FIRST_NAMES)
        } else if (has("last") && has("name"))
            || has("lastname")
            || has("surname")
            || (has("family") && has("name"))
        {
            self.pick(LAST_NAMES)
        } else if has("username")
            || has("login")
            || has("handle")
            || has("nickname")
            || (has("user") && last_tok == "name")
        {
            let first = self.pick(FIRST_NAMES).to_ascii_lowercase();
            format!("{first}{}", self.rng.gen_range(1..1000))
        } else if has("company")
            || has("organization")
            || has("organisation")
            || has("employer")
            || has("business")
            || has("vendor")
            || has("merchant")
        {
            let a = self.pick(LAST_NAMES);
            let b = self.pick(&["Inc", "LLC", "Ltd", "Group", "Labs", "Partners"]);
            format!("{a} {b}")
        } else if has("country") {
            let i = self.rng.gen_range(0..COUNTRIES.len());
            let (code, full) = COUNTRIES[i];
            if has("code") || has("iso") {
                code.to_string()
            } else {
                full.to_string()
            }
        } else if has("currency") {
            self.pick(&["USD", "EUR", "GBP", "CAD", "JPY", "AUD"])
        } else if has("city") || has("town") {
            self.pick(CITIES)
        } else if has("street") || has("address") || has("addr") {
            let n = self.rng.gen_range(1..9999);
            let st = self.pick(LAST_NAMES);
            let kind = self.pick(&["St", "Ave", "Rd", "Blvd", "Ln"]);
            format!("{n} {st} {kind}")
        } else if has("zip") || has("postcode") || has("postal") {
            format!("{:05}", self.rng.gen_range(1001..99950))
        } else if has("language") || has("locale") || has("lang") {
            self.pick(&["en-US", "en-GB", "fr-FR", "de-DE", "es-ES", "ja-JP"])
        } else if has("timezone") || has("tz") {
            self.pick(&[
                "America/New_York",
                "Europe/London",
                "Asia/Tokyo",
                "America/Los_Angeles",
            ])
        } else if has("ip") {
            format!("192.0.2.{}", self.rng.gen_range(1..255))
        } else if has("sku") {
            format!("SKU-{:05}", self.rng.gen_range(0..100000))
        } else if has("color") || has("colour") {
            self.pick(&["red", "blue", "green", "black", "white", "orange"])
        } else if has("title") || has("subject") || has("headline") {
            let a = self.pick(ADJECTIVES);
            let b = self.pick(WORDS);
            let mut t = format!("{a} {b} update");
            t[..1].make_ascii_uppercase();
            t
        } else if has("description")
            || has("comment")
            || has("note")
            || has("notes")
            || has("message")
            || has("summary")
            || has("reason")
            || has("bio")
        {
            let a = self.pick(ADJECTIVES);
            let b = self.pick(WORDS);
            let mut t = format!("{a} {b} looks good");
            t[..1].make_ascii_uppercase();
            t
        } else if has("name") {
            // name, full_name, customer_name, product_name ...
            if has("product") || has("item") {
                let a = self.pick(ADJECTIVES);
                let b = self.pick(WORDS);
                let mut t = format!("{a} {b}");
                t[..1].make_ascii_uppercase();
                t
            } else {
                let f = self.pick(FIRST_NAMES);
                let l = self.pick(LAST_NAMES);
                format!("{f} {l}")
            }
        } else {
            return None;
        };
        Some(v)
    }

    fn alnum(&mut self, len: usize) -> String {
        const CS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        (0..len)
            .map(|_| CS[self.rng.gen_range(0..CS.len())] as char)
            .collect()
    }

    fn lower(&mut self, len: usize) -> String {
        (0..len)
            .map(|_| (b'a' + self.rng.gen_range(0..26)) as char)
            .collect()
    }

    fn date(&mut self) -> String {
        let base = chrono::NaiveDate::from_ymd_opt(2020, 1, 1).expect("valid date");
        let d = base + chrono::Duration::days(self.rng.gen_range(0..4018));
        d.format("%Y-%m-%d").to_string()
    }

    fn timestamp(&mut self) -> String {
        let secs = 1_577_836_800 + self.rng.gen_range(0..347_155_200i64);
        chrono::DateTime::from_timestamp(secs, 0)
            .expect("in range")
            .format("%Y-%m-%dT%H:%M:%SZ")
            .to_string()
    }
}

// ---------------------------------------------------------------------------
// Regex walking
// ---------------------------------------------------------------------------

/// Append a string matching `hir` to `out`. Unbounded repeats add at most
/// `extra` copies past their minimum. Returns false when the expression has
/// no match the generator can build (an empty class, or runaway length).
fn gen_hir(hir: &Hir, rng: &mut StdRng, out: &mut String, extra: u32) -> bool {
    if out.len() > MAX_STRING_LEN {
        return false;
    }
    match hir.kind() {
        HirKind::Empty | HirKind::Look(_) => true,
        HirKind::Literal(lit) => match std::str::from_utf8(&lit.0) {
            Ok(s) => {
                out.push_str(s);
                true
            }
            Err(_) => false,
        },
        HirKind::Class(Class::Unicode(cls)) => {
            let ranges: Vec<(u32, u32)> = cls
                .ranges()
                .iter()
                .map(|r| (r.start() as u32, r.end() as u32))
                .collect();
            match pick_char(&ranges, rng) {
                Some(c) => {
                    out.push(c);
                    true
                }
                None => false,
            }
        }
        HirKind::Class(Class::Bytes(cls)) => {
            let ranges: Vec<(u32, u32)> = cls
                .ranges()
                .iter()
                .map(|r| (r.start() as u32, (r.end() as u32).min(0x7F)))
                .filter(|(a, b)| a <= b)
                .collect();
            match pick_char(&ranges, rng) {
                Some(c) => {
                    out.push(c);
                    true
                }
                None => false,
            }
        }
        HirKind::Repetition(rep) => {
            let min = rep.min;
            let max = rep.max.unwrap_or(min.saturating_add(extra));
            let max = max.min(min.saturating_add(extra.max(1)).max(min));
            let n = if min >= max {
                min
            } else {
                rng.gen_range(min..=max)
            };
            for _ in 0..n {
                if !gen_hir(&rep.sub, rng, out, extra) {
                    return false;
                }
            }
            true
        }
        HirKind::Capture(cap) => gen_hir(&cap.sub, rng, out, extra),
        HirKind::Concat(subs) => subs.iter().all(|s| gen_hir(s, rng, out, extra)),
        HirKind::Alternation(subs) => match subs.choose(rng) {
            Some(s) => gen_hir(s, rng, out, extra),
            None => false,
        },
    }
}

/// Pick a char from inclusive code point ranges, preferring printable ASCII.
fn pick_char(ranges: &[(u32, u32)], rng: &mut StdRng) -> Option<char> {
    let ascii: Vec<(u32, u32)> = ranges
        .iter()
        .map(|&(a, b)| (a.max(0x20), b.min(0x7E)))
        .filter(|(a, b)| a <= b)
        .collect();
    let pool = if ascii.is_empty() {
        ranges.to_vec()
    } else {
        ascii
    };
    let total: u64 = pool.iter().map(|(a, b)| (b - a + 1) as u64).sum();
    if total == 0 {
        return None;
    }
    for _ in 0..16 {
        let mut k = rng.gen_range(0..total);
        for &(a, b) in &pool {
            let size = (b - a + 1) as u64;
            if k < size {
                if let Some(c) = char::from_u32(a + k as u32) {
                    return Some(c);
                }
                break;
            }
            k -= size;
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Contract helpers
// ---------------------------------------------------------------------------

/// Catch rules that contradict each other on their face, with a clear error.
fn precheck_fields(fields: &[FieldDefinition], prefix: &str) -> Result<(), SynthError> {
    for f in fields {
        let path = join(prefix, &f.name);
        if let (Some(lo), Some(hi)) = (f.min, f.max) {
            if lo > hi && matches!(f.field_type, FieldType::Integer | FieldType::Float) {
                return Err(SynthError::new(
                    "min",
                    Some(&path),
                    format!("field '{path}' has min {lo} above max {hi}"),
                ));
            }
        }
        if let (Some(lo), Some(hi)) = (f.min_length, f.max_length) {
            if lo > hi {
                return Err(SynthError::new(
                    "min_length",
                    Some(&path),
                    format!("field '{path}' has min_length {lo} above max_length {hi}"),
                ));
            }
        }
        if let Some(a) = &f.allowed_values {
            if a.is_empty() {
                return Err(SynthError::new(
                    "enum",
                    Some(&path),
                    format!("field '{path}' has an empty enum, so no value can pass"),
                ));
            }
        }
        if let Some(props) = &f.properties {
            precheck_fields(props, &path)?;
        }
    }
    Ok(())
}

fn collect_field_targets(
    fields: &[FieldDefinition],
    prefix: &str,
    in_items: bool,
    out: &mut Vec<Target>,
) {
    for f in fields {
        let p = join(prefix, &f.name);
        field_targets(f, p, f.required, in_items, out);
    }
}

/// Targets for one field at path `p`. `in_items` is true inside array
/// items (rules there are aimed at the first item).
fn field_targets(
    f: &FieldDefinition,
    p: String,
    required: bool,
    in_items: bool,
    out: &mut Vec<Target>,
) {
    {
        if required {
            out.push(Target::Required(p.clone()));
        }
        if f.field_type != FieldType::Any {
            out.push(Target::Type(p.clone()));
        }
        if f.allowed_values.as_ref().is_some_and(|a| !a.is_empty()) {
            out.push(Target::Enum(p.clone()));
        }
        if matches!(f.field_type, FieldType::Integer | FieldType::Float) {
            if f.min.is_some() {
                out.push(Target::Min(p.clone()));
            }
            if f.max.is_some() {
                out.push(Target::Max(p.clone()));
            }
        }
        if is_stringish(&f.field_type) {
            if f.min_length.is_some_and(|m| m >= 1) {
                out.push(Target::MinLength(p.clone()));
            }
            if f.max_length.is_some() {
                out.push(Target::MaxLength(p.clone()));
            }
            if f.pattern.is_some() {
                out.push(Target::Pattern(p.clone()));
            }
            let fmt_enforced = f.field_type != FieldType::String
                || f.format.as_deref().is_some_and(|x| {
                    matches!(
                        x.to_ascii_lowercase().as_str(),
                        "email" | "uuid" | "uri" | "url"
                    )
                });
            if fmt_enforced {
                out.push(Target::Format(p.clone()));
            }
        }
        if f.field_type == FieldType::Object {
            if let Some(props) = &f.properties {
                collect_field_targets(props, &p, in_items, out);
            }
        }
        // Rules inside array items, aimed at the first item.
        if f.field_type == FieldType::Array {
            if let Some(item) = &f.items {
                field_targets(item, format!("{p}[0]"), false, true, out);
            }
        }
    }
}

fn is_stringish(t: &FieldType) -> bool {
    matches!(
        t,
        FieldType::String | FieldType::Date | FieldType::Timestamp
    )
}

fn is_integer_field(def: Option<&FieldDefinition>) -> bool {
    def.is_some_and(|d| d.field_type == FieldType::Integer)
}

/// A string copy of `d` for building a near-miss value of a date or
/// timestamp field (the type check still passes for any string).
fn as_plain_string(d: &FieldDefinition) -> FieldDefinition {
    let mut r = d.clone();
    r.field_type = FieldType::String;
    r.allowed_values = None;
    r
}

fn oracle_key(f: &FieldDefinition) -> String {
    serde_json::to_string(f).unwrap_or_default()
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}.{name}")
    }
}

fn numeric_template(path: &str, min: Option<f64>, max: Option<f64>) -> FieldDefinition {
    let mut t = string_template(path);
    t.field_type = FieldType::Float;
    t.min = min;
    t.max = max;
    t
}

fn string_template(path: &str) -> FieldDefinition {
    FieldDefinition {
        name: path.to_string(),
        field_type: FieldType::String,
        required: true,
        pattern: None,
        allowed_values: None,
        min: None,
        max: None,
        min_length: None,
        max_length: None,
        properties: None,
        items: None,
        transform: None,
        format: None,
    }
}

/// Find the definition at `path`, forcing it and its parents present. A
/// top-level path not in the ontology is added from `template` when
/// `compliance_mode` is off (undeclared fields pass through); otherwise
/// the rule cannot be met.
fn ensure_path<'f>(
    fields: &'f mut Vec<FieldDefinition>,
    path: &str,
    template: FieldDefinition,
    compliance_mode: bool,
    rule: &str,
) -> Result<&'f mut FieldDefinition, SynthError> {
    let mut template = Some(template);
    let mut parts = path.split('.').peekable();
    let mut cur = fields;
    loop {
        let name = parts.next().unwrap_or_default();
        let last = parts.peek().is_none();
        let pos = cur.iter().position(|f| f.name == name);
        let idx = match pos {
            Some(i) => i,
            None if last && !compliance_mode && !path.contains('.') => {
                cur.push(template.take().expect("used once"));
                cur.len() - 1
            }
            None => {
                return Err(SynthError::new(
                    rule,
                    Some(path),
                    format!(
                    "the {rule} rule on '{path}' points at a field the ontology does not declare{}",
                    if compliance_mode {
                        " while compliance_mode rejects undeclared fields"
                    } else {
                        ""
                    }
                ),
                ))
            }
        };
        let f = &mut cur[idx];
        f.required = true;
        if last {
            return Ok(f);
        }
        if f.field_type != FieldType::Object {
            return Err(SynthError::new(
                rule,
                Some(path),
                format!(
                    "the {rule} rule on '{path}' goes through '{}', which is not an object",
                    f.name
                ),
            ));
        }
        cur = f.properties.get_or_insert_with(Vec::new);
    }
}

fn find_def<'f>(fields: &'f [FieldDefinition], path: &str) -> Option<&'f FieldDefinition> {
    let segs = segments(path);
    let mut cur = fields;
    for (n, (name, idx)) in segs.iter().enumerate() {
        let mut f = cur.iter().find(|f| f.name == *name)?;
        if idx.is_some() {
            f = f.items.as_deref()?;
        }
        if n + 1 == segs.len() {
            return Some(f);
        }
        cur = f.properties.as_deref()?;
    }
    None
}

/// Two decimal places; callers step at least 1 past a bound, so rounding
/// never brings a value back inside it.
fn round2(x: f64) -> f64 {
    (x * 100.0).round() / 100.0
}

fn max_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.max(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

fn min_opt(a: Option<f64>, b: Option<f64>) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, None) => x,
        (None, y) => y,
    }
}

/// Integers past 2^53 lose precision as f64 bounds, so stay inside it.
const SAFE_INT: f64 = 9_007_199_254_740_991.0;

fn int_bounds(f: &FieldDefinition, path: &str) -> Result<(i64, i64), SynthError> {
    let (lo, hi) = default_bounds(f.min.map(f64::ceil), f.max.map(f64::floor), 10_000.0);
    let lo = lo.max(-SAFE_INT);
    let hi = hi.min(SAFE_INT);
    if lo > hi {
        return Err(SynthError::new(
            "min",
            Some(path),
            format!(
                "field '{path}' has no integer between min {:?} and max {:?}",
                f.min, f.max
            ),
        ));
    }
    Ok((lo as i64, hi as i64))
}

fn float_bounds(f: &FieldDefinition, path: &str) -> Result<(f64, f64), SynthError> {
    let (lo, hi) = default_bounds(f.min, f.max, 1_000.0);
    if lo > hi || !lo.is_finite() || !hi.is_finite() {
        return Err(SynthError::new(
            "min",
            Some(path),
            format!(
                "field '{path}' has no number between min {:?} and max {:?}",
                f.min, f.max
            ),
        ));
    }
    Ok((lo, hi))
}

fn default_bounds(min: Option<f64>, max: Option<f64>, span: f64) -> (f64, f64) {
    match (min, max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, a + span),
        (None, Some(b)) => (b - span, b),
        (None, None) => (0.0, span),
    }
}

/// Split `a.b[2].c` into `[("a", None), ("b", Some(2)), ("c", None)]`.
fn segments(path: &str) -> Vec<(&str, Option<usize>)> {
    path.split('.')
        .map(
            |seg| match seg.strip_suffix(']').and_then(|s| s.split_once('[')) {
                Some((name, idx)) => (name, idx.parse().ok()),
                None => (seg, None),
            },
        )
        .collect()
}

fn get_path<'v>(v: &'v Value, path: &str) -> Option<&'v Value> {
    let mut cur = v;
    for (name, idx) in segments(path) {
        cur = cur.as_object()?.get(name)?;
        if let Some(i) = idx {
            cur = cur.as_array()?.get(i)?;
        }
    }
    Some(cur)
}

fn get_path_mut<'v>(v: &'v mut Value, segs: &[(&str, Option<usize>)]) -> Option<&'v mut Value> {
    let mut cur = v;
    for (name, idx) in segs {
        cur = cur.as_object_mut()?.get_mut(*name)?;
        if let Some(i) = idx {
            cur = cur.as_array_mut()?.get_mut(*i)?;
        }
    }
    Some(cur)
}

fn set_path(v: &mut Value, path: &str, new: Value) -> Option<()> {
    let segs = segments(path);
    let (last, parents) = segs.split_last()?;
    let parent = get_path_mut(v, parents)?;
    match last {
        (name, None) => {
            parent.as_object_mut()?.insert(name.to_string(), new);
        }
        (name, Some(i)) => {
            *parent
                .as_object_mut()?
                .get_mut(*name)?
                .as_array_mut()?
                .get_mut(*i)? = new;
        }
    }
    Some(())
}

fn remove_path(v: &mut Value, path: &str) {
    let segs = segments(path);
    let Some(((name, None), parents)) = segs.split_last() else {
        return; // array elements are not removed
    };
    if let Some(obj) = get_path_mut(v, parents).and_then(|p| p.as_object_mut()) {
        obj.remove(*name);
    }
}

/// Wrap generated records in the contract's `envelope` (RFC-038) so the
/// result can be posted as one API-style batch. With `validate_wrapper`,
/// the wrapper carries `success` and a one-page `pagination` block.
pub fn wrap_envelope(contract: &Contract, records: Vec<Value>) -> Result<Value, SynthError> {
    let Some(env) = &contract.envelope else {
        return Err(SynthError::new(
            "envelope",
            None,
            "the contract declares no `envelope`, so there is nothing to wrap the rows in",
        ));
    };
    let n = records.len();
    let mut obj = Map::new();
    if env.validate_wrapper {
        obj.insert("success".into(), json!(true));
        obj.insert(
            "pagination".into(),
            json!({"page": 1, "limit": n, "total": n, "hasMore": false}),
        );
    }
    obj.insert(env.records_path.clone(), Value::Array(records));
    Ok(Value::Object(obj))
}

/// Flatten one record into CSV cells: nested objects use dot paths, and
/// arrays (and anything else non-scalar) are written as JSON text.
pub fn flatten_record(v: &Value, prefix: &str, out: &mut Vec<(String, String)>) {
    match v {
        Value::Object(m) if !m.is_empty() || prefix.is_empty() => {
            for (k, child) in m {
                flatten_record(child, &join(prefix, k), out);
            }
        }
        Value::String(s) => out.push((prefix.to_string(), s.clone())),
        Value::Null => out.push((prefix.to_string(), String::new())),
        other => out.push((prefix.to_string(), other.to_string())),
    }
}

/// Is `a` the same path as `b`, or a parent or child of it?
/// Lowercase words of a field name: `customerEmail`, `customer_email` and
/// `customer-email` all give `["customer", "email"]`.
fn name_tokens(name: &str) -> Vec<String> {
    let mut toks = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                toks.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            toks.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        cur.push(ch.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        toks.push(cur);
    }
    toks
}

const FIRST_NAMES: &[&str] = &[
    "Ava", "Liam", "Maya", "Noah", "Zoe", "Ethan", "Priya", "Lucas", "Sofia", "Omar", "Elena",
    "Kenji", "Amara", "Diego", "Hannah", "Ravi", "Chloe", "Mateo", "Leila", "Owen",
];
const LAST_NAMES: &[&str] = &[
    "Garcia", "Smith", "Chen", "Patel", "Johnson", "Okafor", "Nguyen", "Rossi", "Kim", "Lopez",
    "Meyer", "Silva", "Brown", "Tanaka", "Haddad", "Novak", "Walker", "Reyes", "Clark", "Singh",
];
const CITIES: &[&str] = &[
    "Boston", "Austin", "Denver", "Seattle", "Chicago", "Toronto", "London", "Berlin", "Madrid",
    "Tokyo", "Sydney", "Lisbon",
];
const COUNTRIES: &[(&str, &str)] = &[
    ("US", "United States"),
    ("CA", "Canada"),
    ("GB", "United Kingdom"),
    ("DE", "Germany"),
    ("FR", "France"),
    ("ES", "Spain"),
    ("JP", "Japan"),
    ("AU", "Australia"),
    ("BR", "Brazil"),
    ("IN", "India"),
];
const ADJECTIVES: &[&str] = &[
    "quick", "quiet", "bright", "classic", "compact", "daily", "fresh", "simple", "steady", "smart",
];
const WORDS: &[&str] = &[
    "order", "invoice", "report", "shipment", "account", "booking", "widget", "lamp", "backpack",
    "kettle", "notebook", "jacket",
];

fn touches(a: &str, b: &str) -> bool {
    let under = |x: &str, y: &str| {
        x.len() > y.len() && x.starts_with(y) && matches!(x.as_bytes()[y.len()], b'.' | b'[')
    };
    a == b || under(a, b) || under(b, a)
}

fn rfc3339(secs: i64) -> String {
    chrono::DateTime::from_timestamp(secs, 0)
        .unwrap_or_default()
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
}
