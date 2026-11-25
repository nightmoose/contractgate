//! SQL dialect differences for batch contract checks (RFC-119).
//!
//! Range, length, enum, and cross-field comparisons compile to plain ANSI
//! SQL and are identical across Postgres, DuckDB, Snowflake, and
//! Databricks. Regex predicates and "now minus N seconds" arithmetic are
//! the only parts that differ between them; this module isolates those.

use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SqlDialect {
    Postgres,
    DuckDb,
    Snowflake,
    Databricks,
}

impl SqlDialect {
    pub fn as_str(self) -> &'static str {
        match self {
            SqlDialect::Postgres => "postgres",
            SqlDialect::DuckDb => "duckdb",
            SqlDialect::Snowflake => "snowflake",
            SqlDialect::Databricks => "databricks",
        }
    }

    /// All supported dialects, in a stable order used by golden-file tests
    /// and `--dialect all` style enumeration.
    pub fn all() -> [SqlDialect; 4] {
        [
            SqlDialect::Postgres,
            SqlDialect::DuckDb,
            SqlDialect::Snowflake,
            SqlDialect::Databricks,
        ]
    }

    /// `true` for dialects this build can actually run a query against
    /// (`--execute`). Only Postgres is wired up, via the existing `sqlx`
    /// dependency. DuckDB, Snowflake, and Databricks are generation-only:
    /// the `duckdb` crate's bundled build exhausted the shared sandbox disk
    /// during implementation, and there is no Snowflake/Databricks driver
    /// in the dependency allow-list (and no credentials to test one
    /// against). See RFC-119.
    pub fn supports_execute(self) -> bool {
        matches!(self, SqlDialect::Postgres)
    }
}

impl FromStr for SqlDialect {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(SqlDialect::Postgres),
            "duckdb" => Ok(SqlDialect::DuckDb),
            "snowflake" => Ok(SqlDialect::Snowflake),
            "databricks" | "spark" => Ok(SqlDialect::Databricks),
            _ => Err(format!(
                "unknown SQL dialect {s:?}; expected postgres, duckdb, snowflake, or databricks"
            )),
        }
    }
}

impl fmt::Display for SqlDialect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Quote an identifier (table or column name) for this dialect.
pub fn quote_ident(dialect: SqlDialect, name: &str) -> String {
    match dialect {
        SqlDialect::Postgres | SqlDialect::DuckDb | SqlDialect::Snowflake => {
            format!("\"{}\"", name.replace('"', "\"\""))
        }
        SqlDialect::Databricks => format!("`{}`", name.replace('`', "``")),
    }
}

/// Quote a string literal. All four dialects use `'...'` with `''` as the
/// escaped single quote.
pub fn quote_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

/// `true` when `<expr>` matches `pattern` (a contract `pattern:` regex).
/// Callers wrap this in `not (...)` for a violation predicate and handle
/// NULL separately (`is not null and not (...)`).
pub fn regex_match(dialect: SqlDialect, expr: &str, pattern: &str) -> String {
    let lit = quote_literal(pattern);
    match dialect {
        SqlDialect::Postgres => format!("{expr} ~ {lit}"),
        SqlDialect::DuckDb => format!("regexp_matches({expr}, {lit})"),
        SqlDialect::Snowflake => format!("regexp_like({expr}, {lit})"),
        SqlDialect::Databricks => format!("{expr} rlike {lit}"),
    }
}

/// `length(<expr>)` in characters — identical function name across all four.
pub fn length(expr: &str) -> String {
    format!("length({expr})")
}

/// Cast `<expr>` to a string type. Used for `check-table`'s report columns
/// (key values, rendered alongside `rule` and `reason`, which are already
/// text) so a report with a mixed-type key still has one consistent output
/// shape across dialects.
pub fn cast_text(dialect: SqlDialect, expr: &str) -> String {
    match dialect {
        SqlDialect::Postgres | SqlDialect::DuckDb => format!("{expr}::text"),
        SqlDialect::Snowflake => format!("to_varchar({expr})"),
        SqlDialect::Databricks => format!("cast({expr} as string)"),
    }
}

/// Current timestamp expression for this dialect.
pub fn now(dialect: SqlDialect) -> &'static str {
    match dialect {
        SqlDialect::Postgres | SqlDialect::DuckDb => "now()",
        SqlDialect::Snowflake | SqlDialect::Databricks => "current_timestamp()",
    }
}

/// `<now> - <seconds> seconds`, in each dialect's interval syntax.
pub fn now_minus_seconds(dialect: SqlDialect, seconds: u64) -> String {
    let now = now(dialect);
    match dialect {
        SqlDialect::Postgres | SqlDialect::DuckDb => {
            format!("({now} - interval '{seconds} seconds')")
        }
        SqlDialect::Snowflake => format!("dateadd(second, -{seconds}, {now})"),
        SqlDialect::Databricks => format!("({now} - interval {seconds} seconds)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_str() {
        for d in SqlDialect::all() {
            assert_eq!(SqlDialect::from_str(d.as_str()).unwrap(), d);
        }
    }

    #[test]
    fn rejects_unknown_dialect() {
        assert!(SqlDialect::from_str("mysql").is_err());
    }

    #[test]
    fn only_postgres_execute() {
        assert!(SqlDialect::Postgres.supports_execute());
        assert!(!SqlDialect::DuckDb.supports_execute());
        assert!(!SqlDialect::Snowflake.supports_execute());
        assert!(!SqlDialect::Databricks.supports_execute());
    }

    #[test]
    fn literal_escapes_single_quote() {
        assert_eq!(quote_literal("O'Brien"), "'O''Brien'");
    }

    #[test]
    fn databricks_quotes_with_backtick() {
        assert_eq!(
            quote_ident(SqlDialect::Databricks, "order_id"),
            "`order_id`"
        );
        assert_eq!(
            quote_ident(SqlDialect::Postgres, "order_id"),
            "\"order_id\""
        );
    }
}
