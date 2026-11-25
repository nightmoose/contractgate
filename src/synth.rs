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

use crate::contract::{CompareOp, Contract, CrossFieldRule, FieldDefinition, FieldType, QualityRuleType};
use crate::validation::{
    compare_holds, string_format_ok, validate_at, CompiledContract, Violation, ViolationKind,
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
    let contract: Contract = serde_yaml::from_str(yaml)
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
}

impl<'a> Generator<'a> {
    fn new(compiled: &'a CompiledContract, opts: &SynthOptions) -> Result<Self, SynthError> {
        let contract = &compiled.contract;
        precheck_fields(&contract.ontology.entities, "")?;

        let mut eff = contract.ontology.entities.clone();
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
                    Some(row) => {
                        produced = Some(row);
                        break;
                    }
                    None => {
                        dead[idx] = true;
                        skipped.push(SkippedRule {
                            field: t.path().to_string(),
                            rule: t.rule().to_string(),
                            reason: "could not break this rule without also breaking another rule"
                                .into(),
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
    ) -> Result<Option<InvalidRow>, SynthError> {
        for _ in 0..BREAK_ATTEMPTS {
            let base = self.valid_row(true)?;
            let Some(mutated) = self.mutate(base, t, prior) else {
                continue;
            };
            let violations = self.row_violations(&mutated);
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
                return Ok(Some(InvalidRow {
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
        Ok(None)
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
            out.push(Target::CrossField(i, r.field.clone()));
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
                let rule = self.compiled.contract.cross_field[*i].clone();
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
            Target::Type(_) => {
                let d = def?;
                let n = self.rng.gen_range(1..100_000);
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
                let re = self.compiled.patterns.get(p)?.clone();
                let mut relaxed = d.clone();
                relaxed.allowed_values = None;
                relaxed.pattern = None;
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
            Target::Freshness(_, max_age) => {
                let age = *max_age as i64 + 1 + self.rng.gen_range(0..86_400);
                Some(json!(self.now - age))
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
        let rules = self.compiled.contract.cross_field.clone();
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
                    // Swapping an ordered pair breaks lt/lte/gt/gte when the
                    // values differ; otherwise draw new values for `field`.
                    let swapped_breaks = op != CompareOp::Eq && !compare_holds(op, &b, &a);
                    if swapped_breaks {
                        set_path(&mut row, &rule.field, b);
                        set_path(&mut row, other, a);
                    } else {
                        let mut done = false;
                        for _ in 0..8 {
                            let v = self.regen(&rule.field)?;
                            if !compare_holds(op, &v, &b) {
                                set_path(&mut row, &rule.field, v);
                                done = true;
                                break;
                            }
                        }
                        if !done {
                            return None;
                        }
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
                    let local = self.lower(n);
                    let tld = ["com", "org", "net"][self.rng.gen_range(0..3)];
                    Some(format!("{local}@example.{tld}"))
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
        // With a pattern, alternate between the type/format-shaped value
        // and one built from the regex; the oracle keeps whichever fits.
        if let Some(pat) = &f.pattern {
            if semantic.is_none() || attempt % 2 == 1 {
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
/// items, where the validator does not apply `pattern` (patterns are
/// compiled for object paths only), so no pattern target is emitted.
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
            if f.pattern.is_some() && !in_items {
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
        .map(|seg| match seg.strip_suffix(']').and_then(|s| s.split_once('[')) {
            Some((name, idx)) => (name, idx.parse().ok()),
            None => (seg, None),
        })
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
            *parent.as_object_mut()?.get_mut(*name)?.as_array_mut()?.get_mut(*i)? = new;
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
