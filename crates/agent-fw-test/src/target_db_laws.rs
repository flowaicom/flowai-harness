//! TargetDatabase read-only validation proptest laws.
//!
//! Tests that `validate_read_only()` and `ReadOnlyQuery::parse()` correctly
//! accept all read-only queries and reject all mutations, including mutations
//! hidden in subquery positions.
//!
//! # Laws
//!
//! - L1 (Safe Accepted): All SELECT/WITH/EXPLAIN/SHOW queries pass validation
//! - L2 (Mutations Rejected): All INSERT/UPDATE/DELETE/DDL queries fail
//! - L3 (Equivalence): `validate_read_only(sql).is_ok() == ReadOnlyQuery::parse(sql).is_ok()`
//! - L4 (Nested Rejection): Mutations in any subquery position are rejected
//!
//! # Usage
//!
//! ```ignore
//! #[test]
//! fn target_db_laws() {
//!     agent_fw_test::target_db_laws::test_all();
//! }
//! ```

use agent_fw_algebra::target_db::{validate_read_only, ReadOnlyQuery};

// =============================================================================
// Deterministic Law Tests
// =============================================================================

/// Run all deterministic TargetDatabase read-only validation laws.
pub fn test_all() {
    law_select_accepted();
    law_with_accepted();
    law_explain_accepted();
    law_show_accepted();
    law_insert_rejected();
    law_update_rejected();
    law_delete_rejected();
    law_ddl_rejected();
    law_multi_statement_rejected();
    law_for_update_rejected();
    law_select_into_rejected();
    law_smart_constructor_equivalence();
    law_nested_select_accepted();
    law_nested_locking_rejected();
}

// ─── L1: Safe queries accepted ───────────────────────────────────────

/// L1: Basic SELECT queries are accepted.
pub fn law_select_accepted() {
    let queries = [
        "SELECT 1",
        "SELECT * FROM users",
        "SELECT a, b FROM t WHERE x > 1",
        "SELECT COUNT(*) FROM orders GROUP BY status",
        "SELECT * FROM t1 JOIN t2 ON t1.id = t2.fk",
        "SELECT * FROM t ORDER BY id LIMIT 10",
        "SELECT DISTINCT col FROM t",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_ok(),
            "L1: SELECT should be accepted: {sql}"
        );
    }
}

/// L1: WITH (CTE) queries are accepted.
pub fn law_with_accepted() {
    let queries = [
        "WITH cte AS (SELECT 1) SELECT * FROM cte",
        "WITH a AS (SELECT 1), b AS (SELECT 2) SELECT * FROM a, b",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_ok(),
            "L1: WITH should be accepted: {sql}"
        );
    }
}

/// L1: EXPLAIN queries are accepted.
pub fn law_explain_accepted() {
    let queries = ["EXPLAIN SELECT * FROM users", "EXPLAIN ANALYZE SELECT 1"];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_ok(),
            "L1: EXPLAIN should be accepted: {sql}"
        );
    }
}

/// L1: SHOW queries are accepted.
pub fn law_show_accepted() {
    let queries = ["SHOW TABLES", "SHOW COLUMNS FROM users"];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_ok(),
            "L1: SHOW should be accepted: {sql}"
        );
    }
}

// ─── L2: Mutation queries rejected ───────────────────────────────────

/// L2: INSERT is rejected.
pub fn law_insert_rejected() {
    let queries = [
        "INSERT INTO users (name) VALUES ('test')",
        "INSERT INTO t SELECT * FROM other",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_err(),
            "L2: INSERT should be rejected: {sql}"
        );
    }
}

/// L2: UPDATE is rejected.
pub fn law_update_rejected() {
    let queries = [
        "UPDATE users SET name = 'test' WHERE id = 1",
        "UPDATE t SET x = x + 1",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_err(),
            "L2: UPDATE should be rejected: {sql}"
        );
    }
}

/// L2: DELETE is rejected.
pub fn law_delete_rejected() {
    let queries = ["DELETE FROM users WHERE id = 1", "DELETE FROM t"];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_err(),
            "L2: DELETE should be rejected: {sql}"
        );
    }
}

/// L2: DDL is rejected.
pub fn law_ddl_rejected() {
    let queries = [
        "CREATE TABLE t (id INT)",
        "DROP TABLE users",
        "ALTER TABLE t ADD COLUMN c INT",
        "TRUNCATE TABLE users",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_err(),
            "L2: DDL should be rejected: {sql}"
        );
    }
}

/// L2: Multi-statement is rejected.
pub fn law_multi_statement_rejected() {
    assert!(
        validate_read_only("SELECT 1; SELECT 2").is_err(),
        "L2: multi-statement should be rejected"
    );
    assert!(
        validate_read_only("SELECT 1; DROP TABLE users").is_err(),
        "L2: multi-statement with mutation should be rejected"
    );
}

/// L2: FOR UPDATE/SHARE locking clauses are rejected.
pub fn law_for_update_rejected() {
    let queries = [
        "SELECT * FROM users FOR UPDATE",
        "SELECT * FROM users FOR SHARE",
        "SELECT * FROM users FOR UPDATE SKIP LOCKED",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_err(),
            "L2: FOR UPDATE/SHARE should be rejected: {sql}"
        );
    }
}

/// L2: SELECT INTO is rejected.
pub fn law_select_into_rejected() {
    let err = validate_read_only("SELECT * INTO newtable FROM users");
    assert!(err.is_err(), "L2: SELECT INTO should be rejected");
}

// ─── L3: Smart constructor equivalence ───────────────────────────────

/// L3: validate_read_only and ReadOnlyQuery::parse agree on all inputs.
pub fn law_smart_constructor_equivalence() {
    let queries = [
        "SELECT 1",
        "INSERT INTO t VALUES (1)",
        "SELECT * FROM users",
        "DROP TABLE t",
        "UPDATE t SET x = 1",
        "WITH cte AS (SELECT 1) SELECT * FROM cte",
        "DELETE FROM t WHERE id = 1",
        "CREATE TABLE foo (id INT)",
        "EXPLAIN SELECT * FROM t",
        "SHOW TABLES",
        "SELECT * FROM users FOR UPDATE",
        "SELECT * INTO backup FROM users",
        "SELECT 1; SELECT 2",
    ];
    for sql in &queries {
        let validate_result = validate_read_only(sql);
        let parse_result = ReadOnlyQuery::parse(*sql);
        assert_eq!(
            validate_result.is_ok(),
            parse_result.is_ok(),
            "L3: validate_read_only and ReadOnlyQuery::parse must agree on: {sql}"
        );
    }
}

// ─── L4: Nested subquery validation ──────────────────────────────────

/// L4: Nested SELECT subqueries are accepted (read-only at every level).
pub fn law_nested_select_accepted() {
    let queries = [
        "SELECT * FROM (SELECT * FROM users) AS sub",
        "SELECT * FROM t WHERE id IN (SELECT id FROM t)",
        "SELECT * FROM t WHERE EXISTS (SELECT 1 FROM t)",
        "SELECT COALESCE((SELECT 1), 0) FROM t",
        "SELECT CASE WHEN (SELECT 1) = 1 THEN 'yes' ELSE 'no' END FROM t",
        "SELECT * FROM t ORDER BY (SELECT 1)",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_ok(),
            "L4: nested safe SELECT should be accepted: {sql}"
        );
    }
}

/// L4: Locking clauses hidden in subquery positions are rejected.
pub fn law_nested_locking_rejected() {
    let queries = [
        "SELECT * FROM (SELECT * FROM users FOR UPDATE) sub",
        "SELECT * FROM t WHERE id IN (SELECT id FROM t FOR UPDATE)",
        "SELECT * FROM t WHERE EXISTS (SELECT 1 FROM t FOR UPDATE)",
        "SELECT (SELECT 1 FROM t FOR UPDATE) FROM t",
        "SELECT * FROM t JOIN (SELECT * FROM t FOR UPDATE) sub ON true",
        "SELECT COALESCE((SELECT 1 FROM t FOR UPDATE), 0) FROM t",
        "SELECT * FROM t ORDER BY (SELECT 1 FROM t FOR UPDATE)",
        "SELECT * FROM t LIMIT (SELECT 1 FROM t FOR UPDATE)",
        "SELECT * FROM t GROUP BY (SELECT 1 FROM t FOR UPDATE)",
        "SELECT x FROM t GROUP BY x HAVING COUNT(*) > (SELECT 1 FROM t FOR UPDATE)",
    ];
    for sql in &queries {
        assert!(
            validate_read_only(sql).is_err(),
            "L4: locking in subquery should be rejected: {sql}"
        );
    }
}

// =============================================================================
// Proptest Laws
// =============================================================================

#[cfg(test)]
mod hegel_laws {
    use super::*;
    use hegel::generators;

    const SQL_KEYWORDS: &[&str] = &[
        "select",
        "insert",
        "update",
        "delete",
        "drop",
        "create",
        "alter",
        "from",
        "where",
        "into",
        "table",
        "set",
        "values",
        "and",
        "or",
        "not",
        "null",
        "true",
        "false",
        "as",
        "on",
        "join",
        "left",
        "right",
        "inner",
        "outer",
        "group",
        "order",
        "by",
        "having",
        "limit",
        "offset",
        "union",
        "all",
        "distinct",
        "case",
        "when",
        "then",
        "else",
        "end",
        "in",
        "exists",
        "between",
        "like",
        "is",
        "with",
        "recursive",
        "for",
        "share",
        "show",
        "explain",
        "analyze",
        "truncate",
        "grant",
        "revoke",
        "index",
        "view",
    ];

    fn is_keyword(s: &str) -> bool {
        SQL_KEYWORDS.contains(&s.to_lowercase().as_str())
    }

    fn draw_identifier(tc: &hegel::TestCase) -> String {
        loop {
            let s: String = tc.draw(generators::from_regex("[a-z][a-z0-9_]{1,12}").fullmatch(true));
            if !is_keyword(&s) {
                return s;
            }
        }
    }

    fn draw_safe_select(tc: &hegel::TestCase) -> String {
        let num_cols: usize = tc.draw(generators::integers::<usize>().min_value(1).max_value(3));
        let cols: Vec<String> = (0..num_cols).map(|_| draw_identifier(tc)).collect();
        let table = draw_identifier(tc);
        format!("SELECT {} FROM {}", cols.join(", "), table)
    }

    fn draw_safe_select_where(tc: &hegel::TestCase) -> String {
        let col = draw_identifier(tc);
        let table = draw_identifier(tc);
        let wcol = draw_identifier(tc);
        let wval: i32 = tc.draw(generators::integers::<i32>().min_value(1).max_value(999));
        format!("SELECT {} FROM {} WHERE {} > {}", col, table, wcol, wval)
    }

    fn draw_safe_select_join(tc: &hegel::TestCase) -> String {
        let col = draw_identifier(tc);
        let t1 = draw_identifier(tc);
        let t2 = draw_identifier(tc);
        let jcol = draw_identifier(tc);
        format!(
            "SELECT {}.{col} FROM {} JOIN {} ON {}.{jcol} = {}.{jcol}",
            t1, t1, t2, t1, t2
        )
    }

    fn draw_mutation(tc: &hegel::TestCase) -> String {
        let kind: u8 = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
        match kind {
            0 => {
                let table = draw_identifier(tc);
                let col = draw_identifier(tc);
                let val: i32 = tc.draw(generators::integers::<i32>().min_value(1).max_value(99));
                format!("INSERT INTO {} ({}) VALUES ({})", table, col, val)
            }
            1 => {
                let table = draw_identifier(tc);
                let col = draw_identifier(tc);
                let val: i32 = tc.draw(generators::integers::<i32>().min_value(1).max_value(99));
                format!("UPDATE {} SET {} = {}", table, col, val)
            }
            _ => {
                let table = draw_identifier(tc);
                format!("DELETE FROM {}", table)
            }
        }
    }

    fn draw_ddl(tc: &hegel::TestCase) -> String {
        let kind: u8 = tc.draw(generators::integers::<u8>().min_value(0).max_value(2));
        match kind {
            0 => {
                let table = draw_identifier(tc);
                let col = draw_identifier(tc);
                format!("CREATE TABLE {} ({} INT)", table, col)
            }
            1 => {
                let table = draw_identifier(tc);
                format!("DROP TABLE {}", table)
            }
            _ => {
                let table = draw_identifier(tc);
                let col = draw_identifier(tc);
                format!("ALTER TABLE {} ADD COLUMN {} INT", table, col)
            }
        }
    }

    /// L1 (hegel): Random safe SELECT queries are accepted.
    #[hegel::test]
    fn hegel_safe_select_accepted(tc: hegel::TestCase) {
        let sql = draw_safe_select(&tc);
        assert!(
            validate_read_only(&sql).is_ok(),
            "L1 hegel: safe SELECT should be accepted: {}",
            sql
        );
    }

    /// L1 (hegel): Random safe SELECT with WHERE queries are accepted.
    #[hegel::test]
    fn hegel_safe_select_where_accepted(tc: hegel::TestCase) {
        let sql = draw_safe_select_where(&tc);
        assert!(
            validate_read_only(&sql).is_ok(),
            "L1 hegel: safe SELECT WHERE should be accepted: {}",
            sql
        );
    }

    /// L1 (hegel): Random safe SELECT with JOIN queries are accepted.
    #[hegel::test]
    fn hegel_safe_select_join_accepted(tc: hegel::TestCase) {
        let sql = draw_safe_select_join(&tc);
        assert!(
            validate_read_only(&sql).is_ok(),
            "L1 hegel: safe SELECT JOIN should be accepted: {}",
            sql
        );
    }

    /// L2 (hegel): Random mutation queries (INSERT/UPDATE/DELETE) are rejected.
    #[hegel::test]
    fn hegel_mutations_rejected(tc: hegel::TestCase) {
        let sql = draw_mutation(&tc);
        assert!(
            validate_read_only(&sql).is_err(),
            "L2 hegel: mutation should be rejected: {}",
            sql
        );
    }

    /// L2 (hegel): Random DDL queries (CREATE/DROP/ALTER) are rejected.
    #[hegel::test]
    fn hegel_ddl_rejected(tc: hegel::TestCase) {
        let sql = draw_ddl(&tc);
        assert!(
            validate_read_only(&sql).is_err(),
            "L2 hegel: DDL should be rejected: {}",
            sql
        );
    }

    /// L3 (hegel): validate_read_only and ReadOnlyQuery::parse always agree.
    #[hegel::test]
    fn hegel_equivalence_safe(tc: hegel::TestCase) {
        let sql = draw_safe_select(&tc);
        let v = validate_read_only(&sql);
        let p = ReadOnlyQuery::parse(&sql);
        assert_eq!(
            v.is_ok(),
            p.is_ok(),
            "L3 hegel: equivalence violated on safe query: {}",
            sql
        );
    }

    /// L3 (hegel): validate_read_only and ReadOnlyQuery::parse always agree on mutations.
    #[hegel::test]
    fn hegel_equivalence_mutations(tc: hegel::TestCase) {
        let sql = draw_mutation(&tc);
        let v = validate_read_only(&sql);
        let p = ReadOnlyQuery::parse(&sql);
        assert_eq!(
            v.is_ok(),
            p.is_ok(),
            "L3 hegel: equivalence violated on mutation: {}",
            sql
        );
    }

    /// L3 (hegel): validate_read_only and ReadOnlyQuery::parse always agree on DDL.
    #[hegel::test]
    fn hegel_equivalence_ddl(tc: hegel::TestCase) {
        let sql = draw_ddl(&tc);
        let v = validate_read_only(&sql);
        let p = ReadOnlyQuery::parse(&sql);
        assert_eq!(
            v.is_ok(),
            p.is_ok(),
            "L3 hegel: equivalence violated on DDL: {}",
            sql
        );
    }

    /// L4 (hegel): Mutations wrapped in subqueries are still rejected.
    #[hegel::test]
    fn hegel_nested_locking_rejected(tc: hegel::TestCase) {
        let table = draw_identifier(&tc);
        let col = draw_identifier(&tc);
        let sql = format!(
            "SELECT * FROM {} WHERE {} IN (SELECT {} FROM {} FOR UPDATE)",
            table, col, col, table
        );
        assert!(
            validate_read_only(&sql).is_err(),
            "L4 hegel: nested FOR UPDATE should be rejected: {}",
            sql
        );
    }

    #[test]
    fn run_all_deterministic() {
        super::test_all();
    }
}
