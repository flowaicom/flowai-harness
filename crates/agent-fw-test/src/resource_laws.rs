//! Resource bracket and compensating law test harnesses.
//!
//! # Laws
//!
//! - L1 (Bracket releases on success): Release always runs when use succeeds
//! - L2 (Bracket releases on error): Release always runs when use fails
//! - L3 (Use-error preserved): If use fails, its error is returned even though release runs
//! - L4 (Compensating on failure): Compensate runs when action fails
//! - L5 (Compensating skips on success): Compensate does NOT run when action succeeds
//!
//! # Usage
//!
//! ```ignore
//! #[tokio::test]
//! async fn resource_laws_satisfied() {
//!     agent_fw_test::resource_laws::test_all().await;
//! }
//! ```

use agent_fw_algebra::resource::{bracket, compensating};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

/// Run all resource bracket and compensating laws.
pub async fn test_all() {
    law_bracket_releases_on_success().await;
    law_bracket_releases_on_error().await;
    law_use_error_preserved().await;
    law_compensating_on_failure().await;
    law_compensating_skips_on_success().await;
}

/// L1: bracket always releases when use succeeds.
pub async fn law_bracket_releases_on_success() {
    let released = Arc::new(AtomicBool::new(false));
    let released_clone = released.clone();

    let result = bracket(
        async { Ok::<i32, String>(42) },
        |r| Box::pin(async move { Ok(*r * 2) }),
        move |_| {
            released_clone.store(true, Ordering::SeqCst);
            Box::pin(async {})
        },
    )
    .await;

    assert_eq!(result, Ok(84), "L1: use result must be returned");
    assert!(
        released.load(Ordering::SeqCst),
        "L1: release must run on success"
    );
}

/// L2: bracket always releases when use fails.
pub async fn law_bracket_releases_on_error() {
    let released = Arc::new(AtomicBool::new(false));
    let released_clone = released.clone();

    let result: Result<i32, String> = bracket(
        async { Ok::<i32, String>(42) },
        |_| Box::pin(async { Err("use failed".to_string()) }),
        move |_| {
            released_clone.store(true, Ordering::SeqCst);
            Box::pin(async {})
        },
    )
    .await;

    assert!(result.is_err(), "L2: use error must propagate");
    assert!(
        released.load(Ordering::SeqCst),
        "L2: release must run on error"
    );
}

/// L3: If use fails, its error is returned (release errors swallowed).
pub async fn law_use_error_preserved() {
    let result: Result<i32, String> = bracket(
        async { Ok::<i32, String>(42) },
        |_| Box::pin(async { Err("original error".to_string()) }),
        |_| Box::pin(async {}),
    )
    .await;

    assert_eq!(
        result,
        Err("original error".to_string()),
        "L3: use error must be preserved through release"
    );
}

/// L4: compensating runs the compensate closure on action failure.
pub async fn law_compensating_on_failure() {
    let compensated = Arc::new(AtomicBool::new(false));
    let comp = compensated.clone();

    let result = compensating(
        async { Err::<i32, String>("action failed".to_string()) },
        move || async move {
            comp.store(true, Ordering::SeqCst);
        },
    )
    .await;

    assert_eq!(
        result,
        Err("action failed".to_string()),
        "L4: original error must be returned"
    );
    assert!(
        compensated.load(Ordering::SeqCst),
        "L4: compensate must run on failure"
    );
}

/// L5: compensating does NOT run the compensate closure on success.
pub async fn law_compensating_skips_on_success() {
    let compensated = Arc::new(AtomicBool::new(false));
    let comp = compensated.clone();

    let result = compensating(async { Ok::<i32, String>(42) }, move || async move {
        comp.store(true, Ordering::SeqCst);
    })
    .await;

    assert_eq!(result, Ok(42), "L5: success value must be returned");
    assert!(
        !compensated.load(Ordering::SeqCst),
        "L5: compensate must NOT run on success"
    );
}
