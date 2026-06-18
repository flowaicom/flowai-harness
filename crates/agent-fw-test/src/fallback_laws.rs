//! Fallback combinator algebraic law test harnesses.
//!
//! # Laws
//!
//! - L1 (Success passthrough): If primary succeeds, fallback is never called
//! - L2 (Fallback on error): If primary fails, fallback is called with the error
//! - L3 (Source tracking): Result carries provenance (`Primary` | `Fallback`)
//! - L4 (Both errors preserved): If both fail, both errors preserved in `FallbackError`
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn fallback_satisfies_all_laws() {
//!     agent_fw_test::fallback_laws::test_all().await;
//! }
//! ```

use agent_fw_algebra::{with_fallback, with_fallback_sync, FallbackError, FallbackSource};

/// Run all Fallback laws (async + sync variants).
pub async fn test_all() {
    // Async laws
    law_success_passthrough().await;
    law_fallback_on_error().await;
    law_source_tracking().await;
    law_both_errors_preserved().await;
    law_fallback_receives_error().await;

    // Sync laws
    law_sync_success_passthrough();
    law_sync_fallback_on_error();
    law_sync_both_errors_preserved();
}

/// L1: If primary succeeds, fallback is never called.
pub async fn law_success_passthrough() {
    let mut fallback_called = false;
    let result = with_fallback(
        || async { Ok::<i32, String>(42) },
        |_e| {
            fallback_called = true;
            async { Ok(0) }
        },
    )
    .await;

    assert_eq!(
        result,
        Ok((42, FallbackSource::Primary)),
        "L1: primary success must return Primary source"
    );
    assert!(
        !fallback_called,
        "L1: fallback must not be called when primary succeeds"
    );
}

/// L2: If primary fails, fallback is called with the error.
pub async fn law_fallback_on_error() {
    let result = with_fallback(
        || async { Err::<i32, String>("primary failed".into()) },
        |_e| async { Ok(99) },
    )
    .await;

    assert_eq!(
        result,
        Ok((99, FallbackSource::Fallback)),
        "L2: fallback success must return Fallback source"
    );
}

/// L3: Source tracking — Primary vs Fallback tag always matches the path taken.
pub async fn law_source_tracking() {
    // Case 1: primary success → Primary
    let (_, source) = with_fallback(|| async { Ok::<_, String>(1) }, |_| async { Ok(2) })
        .await
        .unwrap();
    assert_eq!(
        source,
        FallbackSource::Primary,
        "L3: source must be Primary on success"
    );

    // Case 2: primary fail, fallback success → Fallback
    let (_, source) = with_fallback(
        || async { Err::<i32, String>("err".into()) },
        |_| async { Ok(2) },
    )
    .await
    .unwrap();
    assert_eq!(
        source,
        FallbackSource::Fallback,
        "L3: source must be Fallback when primary fails"
    );
}

/// L4: If both fail, both errors are preserved in FallbackError.
pub async fn law_both_errors_preserved() {
    let result: Result<(i32, FallbackSource), FallbackError<String>> = with_fallback(
        || async { Err("primary error".into()) },
        |_e| async { Err("fallback error".into()) },
    )
    .await;

    let err = result.expect_err("L4: both failing must return Err");
    assert_eq!(
        err.primary, "primary error",
        "L4: primary error must be preserved"
    );
    assert_eq!(
        err.fallback, "fallback error",
        "L4: fallback error must be preserved"
    );
}

/// Extra: fallback receives the primary error.
pub async fn law_fallback_receives_error() {
    let result = with_fallback(
        || async { Err::<i32, String>("specific error".into()) },
        |e| {
            let msg = e.clone();
            async move { Ok(msg.len() as i32) }
        },
    )
    .await;

    assert_eq!(
        result,
        Ok((14, FallbackSource::Fallback)),
        "fallback must receive the primary error (\"specific error\".len() == 14)"
    );
}

/// L1 sync: primary success skips fallback.
pub fn law_sync_success_passthrough() {
    let result = with_fallback_sync(|| Ok::<_, String>(42), |_| Ok(0));
    assert_eq!(
        result,
        Ok((42, FallbackSource::Primary)),
        "L1 sync: primary success must return Primary source"
    );
}

/// L2 sync: fallback on error.
pub fn law_sync_fallback_on_error() {
    let result = with_fallback_sync(|| Err::<i32, _>("err".to_string()), |_| Ok(99));
    assert_eq!(
        result,
        Ok((99, FallbackSource::Fallback)),
        "L2 sync: fallback success must return Fallback source"
    );
}

/// L4 sync: both errors preserved.
pub fn law_sync_both_errors_preserved() {
    let result: Result<(i32, FallbackSource), FallbackError<String>> =
        with_fallback_sync(|| Err("e1".into()), |_| Err("e2".into()));
    let err = result.expect_err("L4 sync: both failing must return Err");
    assert_eq!(err.primary, "e1", "L4 sync: primary error preserved");
    assert_eq!(err.fallback, "e2", "L4 sync: fallback error preserved");
}
