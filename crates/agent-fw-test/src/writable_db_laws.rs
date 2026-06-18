//! WritableDatabase algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1. Execute-DDL: `execute_ddl(valid_ddl)` succeeds. Invalid DDL is rejected
//!   at `DdlStatement::parse` time before reaching the trait.
//! - L2. Insert-Readable: After `insert_batch(batch)`, rows are retrievable.
//! - L3. Idempotent-Drop: `drop_table_if_exists(t)` on a non-existent table succeeds.
//! - L4. Transaction-Atomicity: `execute_in_transaction(ops)` commits all or rolls back.
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn my_db_satisfies_writable_laws() {
//!     let db = MyWritableDb::new();
//!     agent_fw_test::writable_db_laws::test_all(&db).await;
//! }
//! ```

use agent_fw_algebra::writable_db::{
    DdlStatement, DmlStatement, InsertBatch, TableName, WritableDatabase,
};

/// Run all deterministic WritableDatabase laws.
pub async fn test_all(db: &dyn WritableDatabase) {
    law_execute_ddl(db).await;
    law_idempotent_drop(db).await;
    law_transaction_atomicity(db).await;
    law_health_check(db).await;
    smart_constructor_ddl_rejects_select().await;
    smart_constructor_dml_rejects_ddl().await;
    smart_constructor_batch_rejects_ragged().await;
    smart_constructor_table_name_rejects_injection().await;
}

/// Run insert-readable law (requires a `ReadBack` function to verify rows).
///
/// This is separate from `test_all` because it requires coordination
/// between the writable database and a read path.
pub async fn law_insert_readable<F, Fut>(db: &dyn WritableDatabase, read_rows: F)
where
    F: FnOnce(&str) -> Fut,
    Fut: std::future::Future<Output = Vec<Vec<serde_json::Value>>>,
{
    let ddl =
        DdlStatement::parse("CREATE TABLE law_l2_test (id INT, name TEXT)").expect("Valid DDL");
    db.execute_ddl(&ddl).await.expect("L2: DDL should succeed");

    let batch = InsertBatch::new(
        "law_l2_test",
        vec!["id".into(), "name".into()],
        vec![
            vec![serde_json::json!(1), serde_json::json!("Alice")],
            vec![serde_json::json!(2), serde_json::json!("Bob")],
        ],
    )
    .expect("Valid batch");

    let count = db
        .insert_batch(&batch)
        .await
        .expect("L2: insert should succeed");
    assert_eq!(count, 2, "L2: should insert 2 rows");

    let rows = read_rows("law_l2_test").await;
    assert_eq!(rows.len(), 2, "L2: should read back 2 rows");

    // Clean up
    let table = TableName::parse("law_l2_test").unwrap();
    db.drop_table_if_exists(&table).await.unwrap();
}

/// L1: Execute-DDL — valid DDL succeeds.
pub async fn law_execute_ddl(db: &dyn WritableDatabase) {
    let ddl = DdlStatement::parse("CREATE TABLE law_l1_test (id INT PRIMARY KEY, name TEXT)")
        .expect("Valid DDL");

    db.execute_ddl(&ddl)
        .await
        .expect("L1: valid DDL should succeed");

    // Clean up
    let table = TableName::parse("law_l1_test").unwrap();
    db.drop_table_if_exists(&table).await.unwrap();
}

/// L3: Idempotent-Drop — drop non-existent table succeeds.
pub async fn law_idempotent_drop(db: &dyn WritableDatabase) {
    let table = TableName::parse("law_l3_nonexistent").unwrap();

    // First drop (table doesn't exist) — must succeed
    db.drop_table_if_exists(&table)
        .await
        .expect("L3: drop non-existent table should succeed");

    // Create it, then drop twice
    let ddl = DdlStatement::parse("CREATE TABLE law_l3_nonexistent (id INT)").expect("Valid DDL");
    db.execute_ddl(&ddl)
        .await
        .expect("L3: create should succeed");

    db.drop_table_if_exists(&table)
        .await
        .expect("L3: first drop should succeed");

    db.drop_table_if_exists(&table)
        .await
        .expect("L3: second drop (idempotent) should succeed");
}

/// L4: Transaction Atomicity — execute_in_transaction with valid DML succeeds.
///
/// This tests the happy path. Rollback testing requires interpreter-specific
/// facilities (e.g., `MockWritableDatabase::set_fail_next_transaction`).
pub async fn law_transaction_atomicity(db: &dyn WritableDatabase) {
    let s1 = DmlStatement::parse("INSERT INTO law_l4_test (id) VALUES (1)").expect("Valid DML");
    let s2 = DmlStatement::parse("INSERT INTO law_l4_test (id) VALUES (2)").expect("Valid DML");

    // Transaction with valid statements should succeed
    db.execute_in_transaction(&[s1, s2])
        .await
        .expect("L4: transaction with valid DML should succeed");

    // Empty transaction should also succeed (vacuously)
    db.execute_in_transaction(&[])
        .await
        .expect("L4: empty transaction should succeed");
}

/// Health check should succeed on a valid database.
pub async fn law_health_check(db: &dyn WritableDatabase) {
    db.health_check()
        .await
        .expect("Health check should succeed");
}

// ============================================================================
// Smart constructor validation (these don't need a database instance)
// ============================================================================

/// DdlStatement rejects SELECT.
pub async fn smart_constructor_ddl_rejects_select() {
    let result = DdlStatement::parse("SELECT * FROM foo");
    assert!(result.is_err(), "DdlStatement should reject SELECT");
}

/// DmlStatement rejects DDL.
pub async fn smart_constructor_dml_rejects_ddl() {
    let result = DmlStatement::parse("CREATE TABLE foo (id INT)");
    assert!(result.is_err(), "DmlStatement should reject CREATE TABLE");
}

/// InsertBatch rejects ragged rows.
pub async fn smart_constructor_batch_rejects_ragged() {
    let result = InsertBatch::new(
        "test",
        vec!["a".into(), "b".into()],
        vec![
            vec![serde_json::json!(1), serde_json::json!(2)],
            vec![serde_json::json!(3)], // Only 1 value for 2 columns
        ],
    );
    assert!(result.is_err(), "InsertBatch should reject ragged rows");
}

/// TableName rejects non-identifier characters (positive allowlist).
pub async fn smart_constructor_table_name_rejects_injection() {
    assert!(
        TableName::parse("users; DROP TABLE x").is_err(),
        "TableName should reject semicolons"
    );
    assert!(
        TableName::parse("users--comment").is_err(),
        "TableName should reject dashes"
    );
    assert!(
        TableName::parse("users' OR 1=1").is_err(),
        "TableName should reject quotes"
    );
    assert!(
        TableName::parse("table (").is_err(),
        "TableName should reject parens"
    );
    assert!(
        TableName::parse(".leading_dot").is_err(),
        "TableName should reject leading dot"
    );
}
