//! Composable temporal recurrence schedules.
//!
//! A `Schedule` is a pure value (programs-as-values) describing a temporal pattern.
//! It subsumes `RetryPolicy`'s delay calculation and adds algebraic composition.
//!
//! # Laws
//!
//! - **L1 (Recurrence bound):** `recurs(n).step(n+1, _) == Done`
//! - **L2 (Forever unbounded):** `forever().step(k, _) != Done` for all k
//! - **L3 (Union commutativity):** `a.union(b).step(k,t) == b.union(a).step(k,t)`
//! - **L4 (Intersect commutativity):** `a.intersect(b).step(k,t) == b.intersect(a).step(k,t)`
//! - **L5 (Cap bounded):** `s.capped(d).delay() <= d`
//! - **L6 (UpTo termination):** `s.up_to(d).step(_, elapsed >= d) == Done`
//! - **L7 (Jitter bounded):** jittered delay in `[d/2, d]`
//! - **L8 (Jitter positivity):** positive delay stays positive after jitter
//! - **L9 (Union associativity):** `(a.union(b)).union(c) == a.union(b.union(c))`
//! - **L10 (Intersect associativity):** `(a.intersect(b)).intersect(c) == a.intersect(b.intersect(c))`
//! - **L11 (RetryPolicy bridge):** `Schedule::from_retry_policy(p).step(k, _).delay() == p.delay_for_attempt(k)`
//! - **L12 (Union identity):** `s.union(never()).step(k, t) == s.step(k, t)`
//! - **L13 (Intersect annihilator):** `s.intersect(never()).step(k, t) == Done`

use std::time::{Duration, Instant};

use crate::cancellation::CancellationToken;
use crate::pause::PauseToken;
use crate::retry::RetryPolicy;

/// Decision from a single schedule step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Proceed after this delay.
    Continue(Duration),
    /// Schedule exhausted.
    Done,
}

impl Decision {
    /// Extract the delay if `Continue`, or `None` if `Done`.
    pub fn delay(&self) -> Option<Duration> {
        match self {
            Decision::Continue(d) => Some(*d),
            Decision::Done => None,
        }
    }

    /// Returns `true` if the schedule should continue.
    pub fn is_continue(&self) -> bool {
        matches!(self, Decision::Continue(_))
    }
}

/// A composable temporal recurrence schedule.
///
/// Pure value type — all state is external (attempt number, elapsed time).
/// Constructed via smart constructors and composed via combinators.
#[derive(Clone)]
pub struct Schedule {
    kind: ScheduleKind,
}

#[derive(Clone)]
enum ScheduleKind {
    // Primitives
    Never,
    Once,
    Recurs { max: u32, delays: DelayPattern },
    Forever { delays: DelayPattern },
    // Combinators
    Union(Box<Schedule>, Box<Schedule>),
    Intersect(Box<Schedule>, Box<Schedule>),
    UpTo(Box<Schedule>, Duration),
    Capped(Box<Schedule>, Duration),
    Jittered(Box<Schedule>, u64),
    Delayed(Box<Schedule>, Duration),
}

#[derive(Clone)]
enum DelayPattern {
    Fixed(Duration),
    Exponential {
        base: Duration,
        factor: f64,
        max: Duration,
    },
    Fibonacci {
        one: Duration,
        two: Duration,
        max: Duration,
    },
    Immediate,
}

impl DelayPattern {
    fn delay_for(&self, attempt: u32) -> Duration {
        match self {
            DelayPattern::Fixed(d) => *d,
            DelayPattern::Immediate => Duration::ZERO,
            DelayPattern::Exponential { base, factor, max } => {
                let base_ms = base.as_millis() as f64;
                // Clamp attempt to i32::MAX to prevent wrapping on u32 → i32 cast.
                // For factor > 1.0, any attempt beyond ~60 already saturates to max
                // via f64 infinity → max cap, but we guard the cast regardless.
                let exp = i32::try_from(attempt).unwrap_or(i32::MAX);
                let uncapped_ms = base_ms * factor.powi(exp);
                let max_ms = max.as_millis() as f64;
                let capped_ms = uncapped_ms.min(max_ms);
                if capped_ms.is_finite() && capped_ms >= 0.0 {
                    Duration::from_millis(capped_ms as u64)
                } else {
                    Duration::ZERO
                }
            }
            DelayPattern::Fibonacci { one, two, max } => {
                if attempt == 0 {
                    return (*one).min(*max);
                }
                if attempt == 1 {
                    return (*two).min(*max);
                }
                let mut a = one.as_millis() as u64;
                let mut b = two.as_millis() as u64;
                for _ in 2..=attempt {
                    let next = a.saturating_add(b);
                    a = b;
                    b = next;
                }
                Duration::from_millis(b.min(max.as_millis() as u64))
            }
        }
    }
}

// ─── Smart Constructors ──────────────────────────────────────────────

impl Schedule {
    /// Zero executions. Identity for `union`, annihilator for `intersect`.
    ///
    /// # Laws
    ///
    /// - **L12 (Union identity):** `s.union(never()) == s`
    /// - **L13 (Intersect annihilator):** `s.intersect(never()) == never()`
    pub fn never() -> Self {
        Self {
            kind: ScheduleKind::Never,
        }
    }

    /// Single execution, no delay.
    pub fn once() -> Self {
        Self {
            kind: ScheduleKind::Once,
        }
    }

    /// `n + 1` total attempts (n retries) with zero delay.
    pub fn recurs(n: u32) -> Self {
        Self {
            kind: ScheduleKind::Recurs {
                max: n,
                delays: DelayPattern::Immediate,
            },
        }
    }

    /// Unbounded recurrence with zero delay.
    pub fn forever() -> Self {
        Self {
            kind: ScheduleKind::Forever {
                delays: DelayPattern::Immediate,
            },
        }
    }

    /// Fixed interval, unbounded.
    pub fn spaced(interval: Duration) -> Self {
        Self {
            kind: ScheduleKind::Forever {
                delays: DelayPattern::Fixed(interval),
            },
        }
    }

    /// Default cap for exponential/fibonacci delays (1 hour).
    const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(3600);

    /// Exponential backoff, unbounded. Individual delays cap at 1 hour.
    ///
    /// Use `.with_max_delay(d)` to override the cap, or `.capped(d)` to
    /// apply a global cap to any schedule.
    pub fn exponential(base: Duration, factor: f64) -> Self {
        Self::exponential_with_max(base, factor, Self::DEFAULT_MAX_DELAY)
    }

    /// Exponential backoff with explicit per-step delay cap.
    pub fn exponential_with_max(base: Duration, factor: f64, max_delay: Duration) -> Self {
        Self {
            kind: ScheduleKind::Forever {
                delays: DelayPattern::Exponential {
                    base,
                    factor,
                    max: max_delay,
                },
            },
        }
    }

    /// Fibonacci delays, unbounded. Individual delays cap at 1 hour.
    ///
    /// Use `.with_max_delay(d)` to override the cap, or `.capped(d)` to
    /// apply a global cap to any schedule.
    pub fn fibonacci(one: Duration, two: Duration) -> Self {
        Self::fibonacci_with_max(one, two, Self::DEFAULT_MAX_DELAY)
    }

    /// Fibonacci delays with explicit per-step delay cap.
    pub fn fibonacci_with_max(one: Duration, two: Duration, max_delay: Duration) -> Self {
        Self {
            kind: ScheduleKind::Forever {
                delays: DelayPattern::Fibonacci {
                    one,
                    two,
                    max: max_delay,
                },
            },
        }
    }

    /// Bridge from existing `RetryPolicy`.
    ///
    /// The resulting schedule has `max_retries` recurrences and produces
    /// the same delays as `policy.delay_for_attempt(k)` for each attempt k.
    pub fn from_retry_policy(policy: &RetryPolicy) -> Self {
        let delays = DelayPattern::Exponential {
            base: policy.initial_delay(),
            factor: policy.multiplier(),
            max: policy.max_delay(),
        };

        let base = Self {
            kind: ScheduleKind::Recurs {
                max: policy.max_retries(),
                delays,
            },
        };

        if policy.jitter() {
            base.jittered()
        } else {
            base
        }
    }

    // ─── Combinators ─────────────────────────────────────────────────

    /// Continue if either schedule continues; shorter delay wins.
    pub fn union(self, other: Schedule) -> Self {
        Self {
            kind: ScheduleKind::Union(Box::new(self), Box::new(other)),
        }
    }

    /// Continue only if both schedules continue; longer delay wins.
    pub fn intersect(self, other: Schedule) -> Self {
        Self {
            kind: ScheduleKind::Intersect(Box::new(self), Box::new(other)),
        }
    }

    /// Bound this schedule to n attempts (n+1 total including attempt 0).
    pub fn take(self, n: u32) -> Self {
        self.intersect(Schedule::recurs(n))
    }

    /// Stop after total elapsed time exceeds the given duration.
    pub fn up_to(self, duration: Duration) -> Self {
        Self {
            kind: ScheduleKind::UpTo(Box::new(self), duration),
        }
    }

    /// Cap individual delays to the given maximum.
    pub fn capped(self, max: Duration) -> Self {
        Self {
            kind: ScheduleKind::Capped(Box::new(self), max),
        }
    }

    /// Apply `[d/2, d]` jitter to delays.
    ///
    /// The jitter seed is captured at construction time via `RandomState`,
    /// making each schedule instance deterministic (referential transparency):
    /// `s.step(k, t) == s.step(k, t)` always holds.
    pub fn jittered(self) -> Self {
        use std::collections::hash_map::RandomState;
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = RandomState::new().build_hasher();
        hasher.write_u64(0xDEAD_BEEF_CAFE_BABE);
        let seed = hasher.finish();
        Self {
            kind: ScheduleKind::Jittered(Box::new(self), seed),
        }
    }

    /// Add an initial delay before the first step only.
    pub fn delayed(self, d: Duration) -> Self {
        Self {
            kind: ScheduleKind::Delayed(Box::new(self), d),
        }
    }

    // ─── Core Step Function ──────────────────────────────────────────

    /// Pure step function: given attempt number and elapsed time, produce a decision.
    ///
    /// Stateless — all state is in `(attempt, elapsed)`.
    pub fn step(&self, attempt: u32, elapsed: Duration) -> Decision {
        match &self.kind {
            ScheduleKind::Never => Decision::Done,
            ScheduleKind::Once => {
                if attempt == 0 {
                    Decision::Continue(Duration::ZERO)
                } else {
                    Decision::Done
                }
            }
            ScheduleKind::Recurs { max, delays } => {
                if attempt > *max {
                    Decision::Done
                } else {
                    Decision::Continue(delays.delay_for(attempt))
                }
            }
            ScheduleKind::Forever { delays } => Decision::Continue(delays.delay_for(attempt)),
            ScheduleKind::Union(a, b) => {
                let da = a.step(attempt, elapsed);
                let db = b.step(attempt, elapsed);
                match (da, db) {
                    (Decision::Continue(a_delay), Decision::Continue(b_delay)) => {
                        Decision::Continue(a_delay.min(b_delay))
                    }
                    (Decision::Continue(d), Decision::Done) => Decision::Continue(d),
                    (Decision::Done, Decision::Continue(d)) => Decision::Continue(d),
                    (Decision::Done, Decision::Done) => Decision::Done,
                }
            }
            ScheduleKind::Intersect(a, b) => {
                let da = a.step(attempt, elapsed);
                let db = b.step(attempt, elapsed);
                match (da, db) {
                    (Decision::Continue(a_delay), Decision::Continue(b_delay)) => {
                        Decision::Continue(a_delay.max(b_delay))
                    }
                    _ => Decision::Done,
                }
            }
            ScheduleKind::UpTo(inner, max_elapsed) => {
                if elapsed >= *max_elapsed {
                    Decision::Done
                } else {
                    inner.step(attempt, elapsed)
                }
            }
            ScheduleKind::Capped(inner, max_delay) => match inner.step(attempt, elapsed) {
                Decision::Continue(d) => Decision::Continue(d.min(*max_delay)),
                Decision::Done => Decision::Done,
            },
            ScheduleKind::Jittered(inner, seed) => {
                match inner.step(attempt, elapsed) {
                    Decision::Continue(d) => {
                        let ms = d.as_millis() as u64;
                        if ms == 0 {
                            return Decision::Continue(Duration::ZERO);
                        }
                        let half = ms / 2;
                        let range = ms - half + 1; // always >= 1
                        let hash = jitter_hash(*seed, ms, attempt);
                        let jittered = half + (hash % range);
                        // Positivity: positive delay stays positive after jitter
                        let floored = if ms > 0 { jittered.max(1) } else { 0 };
                        Decision::Continue(Duration::from_millis(floored))
                    }
                    Decision::Done => Decision::Done,
                }
            }
            ScheduleKind::Delayed(inner, initial_delay) => match inner.step(attempt, elapsed) {
                Decision::Continue(d) if attempt == 0 => Decision::Continue(d + *initial_delay),
                other => other,
            },
        }
    }
}

/// Deterministic jitter hash (SplitMix64-style mixing).
///
/// Given a seed (captured at construction), delay_ms, and attempt,
/// produces a deterministic u64. Same inputs always yield same output.
fn jitter_hash(seed: u64, delay_ms: u64, attempt: u32) -> u64 {
    let mut z = seed
        .wrapping_add(delay_ms.wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .wrapping_add(attempt as u64);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

impl std::fmt::Debug for Schedule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        fn fmt_kind(kind: &ScheduleKind, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match kind {
                ScheduleKind::Never => f.write_str("Schedule::never()"),
                ScheduleKind::Once => f.write_str("Schedule::once()"),
                ScheduleKind::Recurs { max, delays } => {
                    write!(f, "Schedule::recurs({max})")?;
                    fmt_delays(delays, f)
                }
                ScheduleKind::Forever { delays } => match delays {
                    DelayPattern::Immediate => f.write_str("Schedule::forever()"),
                    DelayPattern::Fixed(d) => write!(f, "Schedule::spaced({d:?})"),
                    _ => {
                        f.write_str("Schedule::forever()")?;
                        fmt_delays(delays, f)
                    }
                },
                ScheduleKind::Union(a, b) => {
                    fmt_kind(&a.kind, f)?;
                    f.write_str(".union(")?;
                    fmt_kind(&b.kind, f)?;
                    f.write_str(")")
                }
                ScheduleKind::Intersect(a, b) => {
                    fmt_kind(&a.kind, f)?;
                    f.write_str(".intersect(")?;
                    fmt_kind(&b.kind, f)?;
                    f.write_str(")")
                }
                ScheduleKind::UpTo(inner, d) => {
                    fmt_kind(&inner.kind, f)?;
                    write!(f, ".up_to({d:?})")
                }
                ScheduleKind::Capped(inner, d) => {
                    fmt_kind(&inner.kind, f)?;
                    write!(f, ".capped({d:?})")
                }
                ScheduleKind::Jittered(inner, _seed) => {
                    fmt_kind(&inner.kind, f)?;
                    f.write_str(".jittered()")
                }
                ScheduleKind::Delayed(inner, d) => {
                    fmt_kind(&inner.kind, f)?;
                    write!(f, ".delayed({d:?})")
                }
            }
        }

        fn fmt_delays(delays: &DelayPattern, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match delays {
                DelayPattern::Immediate => Ok(()),
                DelayPattern::Fixed(d) => write!(f, "[fixed {d:?}]"),
                DelayPattern::Exponential { base, factor, max } => {
                    write!(f, "[exp {base:?}*{factor}, max={max:?}]")
                }
                DelayPattern::Fibonacci { one, two, max } => {
                    write!(f, "[fib {one:?},{two:?}, max={max:?}]")
                }
            }
        }

        fmt_kind(&self.kind, f)
    }
}

// ─── Outcome Types ───────────────────────────────────────────────────

/// Outcome of a schedule-driven execution.
pub struct ScheduleOutcome<T> {
    /// The value produced by the final attempt.
    pub value: T,
    /// Number of attempts performed.
    pub attempts: u32,
    /// Total elapsed wall-clock time (including paused time).
    pub total_elapsed: Duration,
    /// Time spent paused (excluded from schedule delay accounting).
    pub paused_time: Duration,
    /// Whether execution was cancelled.
    pub cancelled: bool,
}

// ─── pausable_sleep ──────────────────────────────────────────────────

/// Outcome of `pausable_sleep`, with precise pause time measurement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PausableSleepOutcome {
    /// `true` if the full duration elapsed, `false` if cancelled.
    pub completed: bool,
    /// Time spent paused during this sleep, measured precisely with
    /// `Instant` around `wait_if_paused` (not derived from OS jitter).
    pub paused_time: Duration,
}

/// Sleep for `duration`, freezing the timer while paused.
///
/// Paused time does NOT count toward the duration. If cancelled,
/// returns immediately with `completed: false`.
///
/// Returns a `PausableSleepOutcome` with precise `paused_time` measurement.
///
/// # Laws
///
/// - **L1 (Freeze):** Paused time does not count toward duration
/// - **L2 (Cancel interrupts):** Returns completed=false on cancellation at any point
/// - **L3 (Zero passthrough):** `pausable_sleep(ZERO, _, _)` returns completed=true immediately
/// - **L4 (Equivalence):** With never-paused token, behaves like `tokio::time::sleep`
pub async fn pausable_sleep(
    duration: Duration,
    cancel: &CancellationToken,
    pause: &PauseToken,
) -> PausableSleepOutcome {
    let mut remaining = duration;
    let mut paused_time = Duration::ZERO;
    loop {
        if remaining.is_zero() {
            return PausableSleepOutcome {
                completed: true,
                paused_time,
            };
        }
        let before = Instant::now();
        tokio::select! {
            () = tokio::time::sleep(remaining) => {
                return PausableSleepOutcome { completed: true, paused_time };
            }
            () = cancel.cancelled() => {
                return PausableSleepOutcome { completed: false, paused_time };
            }
            () = pause.until_paused() => {
                // Freeze: subtract elapsed from remaining
                remaining = remaining.saturating_sub(before.elapsed());
                // Measure pause time precisely
                let pause_start = Instant::now();
                tokio::select! {
                    () = pause.wait_if_paused() => {
                        paused_time += pause_start.elapsed();
                    } // resumed, loop back
                    () = cancel.cancelled() => {
                        paused_time += pause_start.elapsed();
                        return PausableSleepOutcome { completed: false, paused_time };
                    }
                }
            }
        }
    }
}

// ─── Execution Combinators ───────────────────────────────────────────

/// Repeat an infallible operation on a schedule.
///
/// Runs `operation` once per schedule step. Between steps, sleeps for the
/// schedule's delay using `pausable_sleep` (which freezes during pause and
/// races against cancellation).
///
/// Returns `None` if the schedule produces zero steps (e.g., `Schedule::never()`).
/// This makes the function total — no panic on empty schedules.
pub async fn repeat_on_schedule<F, Fut, T>(
    schedule: &Schedule,
    cancel: &CancellationToken,
    pause: &PauseToken,
    mut operation: F,
) -> Option<ScheduleOutcome<T>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let start = Instant::now();
    let mut paused_total = Duration::ZERO;
    let mut attempt = 0u32;
    let mut last_value: Option<T> = None;

    loop {
        let step_elapsed = start.elapsed();
        match schedule.step(attempt, step_elapsed) {
            Decision::Done => break,
            Decision::Continue(delay) => {
                // Execute the operation
                let value = operation().await;
                last_value = Some(value);
                attempt += 1;

                // Check if next step would be Done (avoid sleeping for nothing)
                if matches!(schedule.step(attempt, start.elapsed()), Decision::Done) {
                    break;
                }

                // Sleep between attempts (pausable + cancellable)
                if !delay.is_zero() {
                    let sleep_outcome = pausable_sleep(delay, cancel, pause).await;
                    paused_total += sleep_outcome.paused_time;
                    if !sleep_outcome.completed {
                        return Some(ScheduleOutcome {
                            value: last_value.unwrap(),
                            attempts: attempt,
                            total_elapsed: start.elapsed(),
                            paused_time: paused_total,
                            cancelled: true,
                        });
                    }
                }

                // Check cancellation (no post-cancel execution)
                if cancel.is_cancelled() {
                    return Some(ScheduleOutcome {
                        value: last_value.unwrap(),
                        attempts: attempt,
                        total_elapsed: start.elapsed(),
                        paused_time: paused_total,
                        cancelled: true,
                    });
                }
            }
        }
    }

    last_value.map(|value| ScheduleOutcome {
        value,
        attempts: attempt,
        total_elapsed: start.elapsed(),
        paused_time: paused_total,
        cancelled: false,
    })
}

/// Retry a fallible operation on a schedule.
///
/// Retries on `Err` until the schedule is exhausted or the operation succeeds.
/// `Done` means stop — no final attempt after exhaustion.
/// Cancellation means stop — no post-cancel execution.
///
/// Returns `None` if the schedule produces zero steps (e.g., `Schedule::never()`).
pub async fn retry_on_schedule<F, Fut, T, E>(
    schedule: &Schedule,
    cancel: &CancellationToken,
    pause: &PauseToken,
    mut operation: F,
) -> Option<ScheduleOutcome<Result<T, E>>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
{
    let start = Instant::now();
    let mut paused_total = Duration::ZERO;
    let mut attempt = 0u32;
    let mut last_value: Option<Result<T, E>> = None;

    loop {
        let step_elapsed = start.elapsed();
        match schedule.step(attempt, step_elapsed) {
            Decision::Done => break,
            Decision::Continue(delay) => {
                let value = operation().await;
                let is_ok = value.is_ok();
                last_value = Some(value);
                attempt += 1;

                // Success: return immediately
                if is_ok {
                    return Some(ScheduleOutcome {
                        value: last_value.unwrap(),
                        attempts: attempt,
                        total_elapsed: start.elapsed(),
                        paused_time: paused_total,
                        cancelled: false,
                    });
                }

                // Check if next step would be Done (avoid sleeping for nothing)
                if matches!(schedule.step(attempt, start.elapsed()), Decision::Done) {
                    break;
                }

                // Sleep between attempts
                if !delay.is_zero() {
                    let sleep_outcome = pausable_sleep(delay, cancel, pause).await;
                    paused_total += sleep_outcome.paused_time;
                    if !sleep_outcome.completed {
                        return Some(ScheduleOutcome {
                            value: last_value.unwrap(),
                            attempts: attempt,
                            total_elapsed: start.elapsed(),
                            paused_time: paused_total,
                            cancelled: true,
                        });
                    }
                }

                // Check cancellation (no post-cancel execution)
                if cancel.is_cancelled() {
                    return Some(ScheduleOutcome {
                        value: last_value.unwrap(),
                        attempts: attempt,
                        total_elapsed: start.elapsed(),
                        paused_time: paused_total,
                        cancelled: true,
                    });
                }
            }
        }
    }

    last_value.map(|value| ScheduleOutcome {
        value,
        attempts: attempt,
        total_elapsed: start.elapsed(),
        paused_time: paused_total,
        cancelled: false,
    })
}

/// Retry with a predicate controlling which errors are retryable.
///
/// `Done` means stop — no final attempt after exhaustion.
/// Cancellation means stop — no post-cancel execution.
///
/// Returns `None` if the schedule produces zero steps.
pub async fn retry_on_schedule_when<F, Fut, T, E, P>(
    schedule: &Schedule,
    cancel: &CancellationToken,
    pause: &PauseToken,
    mut operation: F,
    mut should_retry: P,
) -> Option<ScheduleOutcome<Result<T, E>>>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, E>>,
    P: FnMut(&E) -> bool,
{
    let start = Instant::now();
    let mut paused_total = Duration::ZERO;
    let mut attempt = 0u32;
    let mut last_value: Option<Result<T, E>> = None;

    loop {
        let step_elapsed = start.elapsed();
        match schedule.step(attempt, step_elapsed) {
            Decision::Done => break,
            Decision::Continue(delay) => {
                let value = operation().await;
                let should_stop = match &value {
                    Ok(_) => true,
                    Err(e) if !should_retry(e) => true,
                    Err(_) => false,
                };
                last_value = Some(value);
                attempt += 1;

                if should_stop {
                    return Some(ScheduleOutcome {
                        value: last_value.unwrap(),
                        attempts: attempt,
                        total_elapsed: start.elapsed(),
                        paused_time: paused_total,
                        cancelled: false,
                    });
                }

                // Check if next step would be Done
                if matches!(schedule.step(attempt, start.elapsed()), Decision::Done) {
                    break;
                }

                if !delay.is_zero() {
                    let sleep_outcome = pausable_sleep(delay, cancel, pause).await;
                    paused_total += sleep_outcome.paused_time;
                    if !sleep_outcome.completed {
                        return Some(ScheduleOutcome {
                            value: last_value.unwrap(),
                            attempts: attempt,
                            total_elapsed: start.elapsed(),
                            paused_time: paused_total,
                            cancelled: true,
                        });
                    }
                }

                // Check cancellation (no post-cancel execution)
                if cancel.is_cancelled() {
                    return Some(ScheduleOutcome {
                        value: last_value.unwrap(),
                        attempts: attempt,
                        total_elapsed: start.elapsed(),
                        paused_time: paused_total,
                        cancelled: true,
                    });
                }
            }
        }
    }

    last_value.map(|value| ScheduleOutcome {
        value,
        attempts: attempt,
        total_elapsed: start.elapsed(),
        paused_time: paused_total,
        cancelled: false,
    })
}

// ─── Tests ───────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Schedule step tests ──

    #[test]
    fn once_produces_single_step() {
        let s = Schedule::once();
        assert_eq!(
            s.step(0, Duration::ZERO),
            Decision::Continue(Duration::ZERO)
        );
        assert_eq!(s.step(1, Duration::ZERO), Decision::Done);
    }

    #[test]
    fn recurs_bound() {
        let s = Schedule::recurs(3);
        for k in 0..=3 {
            assert!(
                s.step(k, Duration::ZERO).is_continue(),
                "attempt {k} should continue"
            );
        }
        assert_eq!(s.step(4, Duration::ZERO), Decision::Done);
    }

    #[test]
    fn forever_unbounded() {
        let s = Schedule::forever();
        for k in 0..1000 {
            assert!(
                s.step(k, Duration::ZERO).is_continue(),
                "attempt {k} should continue"
            );
        }
    }

    #[test]
    fn spaced_fixed_interval() {
        let s = Schedule::spaced(Duration::from_millis(100));
        for k in 0..10 {
            assert_eq!(
                s.step(k, Duration::ZERO),
                Decision::Continue(Duration::from_millis(100))
            );
        }
    }

    #[test]
    fn exponential_growth() {
        let s = Schedule::exponential(Duration::from_millis(100), 2.0);
        assert_eq!(
            s.step(0, Duration::ZERO).delay(),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            s.step(1, Duration::ZERO).delay(),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            s.step(2, Duration::ZERO).delay(),
            Some(Duration::from_millis(400))
        );
    }

    #[test]
    fn fibonacci_delays() {
        let s = Schedule::fibonacci(Duration::from_millis(100), Duration::from_millis(100));
        // fib: 100, 100, 200, 300, 500, 800
        assert_eq!(
            s.step(0, Duration::ZERO).delay(),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            s.step(1, Duration::ZERO).delay(),
            Some(Duration::from_millis(100))
        );
        assert_eq!(
            s.step(2, Duration::ZERO).delay(),
            Some(Duration::from_millis(200))
        );
        assert_eq!(
            s.step(3, Duration::ZERO).delay(),
            Some(Duration::from_millis(300))
        );
        assert_eq!(
            s.step(4, Duration::ZERO).delay(),
            Some(Duration::from_millis(500))
        );
    }

    #[test]
    fn union_min_delay() {
        let a = Schedule::spaced(Duration::from_millis(100));
        let b = Schedule::spaced(Duration::from_millis(200));
        let u = a.union(b);
        assert_eq!(
            u.step(0, Duration::ZERO).delay(),
            Some(Duration::from_millis(100))
        );
    }

    #[test]
    fn intersect_max_delay() {
        let a = Schedule::spaced(Duration::from_millis(100));
        let b = Schedule::spaced(Duration::from_millis(200));
        let i = a.intersect(b);
        assert_eq!(
            i.step(0, Duration::ZERO).delay(),
            Some(Duration::from_millis(200))
        );
    }

    #[test]
    fn union_extends_with_done() {
        let finite = Schedule::recurs(1); // attempts 0, 1
        let infinite = Schedule::forever();
        let u = finite.union(infinite);
        // After finite is Done, infinite keeps going
        assert!(u.step(5, Duration::ZERO).is_continue());
    }

    #[test]
    fn intersect_terminates_with_done() {
        let finite = Schedule::recurs(1); // attempts 0, 1
        let infinite = Schedule::forever();
        let i = finite.intersect(infinite);
        assert!(i.step(0, Duration::ZERO).is_continue());
        assert!(i.step(1, Duration::ZERO).is_continue());
        assert_eq!(i.step(2, Duration::ZERO), Decision::Done);
    }

    #[test]
    fn up_to_terminates_on_elapsed() {
        let s = Schedule::forever().up_to(Duration::from_secs(10));
        assert!(s.step(0, Duration::from_secs(5)).is_continue());
        assert_eq!(s.step(0, Duration::from_secs(10)), Decision::Done);
        assert_eq!(s.step(0, Duration::from_secs(15)), Decision::Done);
    }

    #[test]
    fn capped_bounds_delay() {
        let s = Schedule::exponential(Duration::from_millis(100), 2.0)
            .capped(Duration::from_millis(300));
        // attempt 0: 100, attempt 1: 200, attempt 2: 400 -> capped to 300
        assert_eq!(
            s.step(2, Duration::ZERO).delay(),
            Some(Duration::from_millis(300))
        );
    }

    #[test]
    fn jittered_in_half_to_full_range() {
        let s = Schedule::spaced(Duration::from_millis(200)).jittered();
        for _ in 0..50 {
            let d = s.step(0, Duration::ZERO).delay().unwrap();
            assert!(d >= Duration::from_millis(100), "jitter too low: {d:?}");
            assert!(d <= Duration::from_millis(200), "jitter too high: {d:?}");
        }
    }

    #[test]
    fn jittered_preserves_zero() {
        let s = Schedule::forever().jittered(); // zero delay
        assert_eq!(s.step(0, Duration::ZERO).delay(), Some(Duration::ZERO));
    }

    #[test]
    fn jitter_positivity() {
        let s = Schedule::spaced(Duration::from_millis(1)).jittered();
        for _ in 0..100 {
            let d = s.step(0, Duration::ZERO).delay().unwrap();
            assert!(
                d >= Duration::from_millis(1),
                "positive delay must stay positive: {d:?}"
            );
        }
    }

    #[test]
    fn delayed_adds_initial_delay() {
        let s = Schedule::spaced(Duration::from_millis(100)).delayed(Duration::from_millis(500));
        assert_eq!(
            s.step(0, Duration::ZERO).delay(),
            Some(Duration::from_millis(600))
        );
        assert_eq!(
            s.step(1, Duration::ZERO).delay(),
            Some(Duration::from_millis(100))
        );
    }

    #[test]
    fn take_limits_recurrence() {
        let s = Schedule::forever().take(2);
        assert!(s.step(0, Duration::ZERO).is_continue());
        assert!(s.step(1, Duration::ZERO).is_continue());
        assert!(s.step(2, Duration::ZERO).is_continue());
        assert_eq!(s.step(3, Duration::ZERO), Decision::Done);
    }

    #[test]
    fn from_retry_policy_no_jitter() {
        let policy = RetryPolicy::exponential_backoff(3, Duration::from_millis(100));
        let schedule = Schedule::from_retry_policy(&policy);
        for k in 0..=3 {
            let sd = schedule.step(k, Duration::ZERO).delay();
            let pd = policy.delay_for_attempt(k);
            assert_eq!(sd, Some(pd), "mismatch at attempt {k}");
        }
        assert_eq!(schedule.step(4, Duration::ZERO), Decision::Done);
    }

    // ── pausable_sleep tests ──

    #[tokio::test]
    async fn pausable_sleep_zero_passthrough() {
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let outcome = pausable_sleep(Duration::ZERO, &cancel, &pause).await;
        assert!(outcome.completed);
    }

    #[tokio::test]
    async fn pausable_sleep_cancelled() {
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        cancel.cancel();
        let outcome = pausable_sleep(Duration::from_secs(10), &cancel, &pause).await;
        assert!(!outcome.completed);
    }

    #[tokio::test]
    async fn pausable_sleep_completes_normally() {
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            pausable_sleep(Duration::from_millis(10), &cancel, &pause),
        )
        .await
        .expect("should complete");
        assert!(outcome.completed);
    }

    // ── Execution combinator tests ──

    #[tokio::test]
    async fn repeat_on_schedule_runs_correct_count() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let count = Arc::new(AtomicU32::new(0));
        let count_clone = count.clone();

        let schedule = Schedule::recurs(2); // 3 total attempts (0, 1, 2)
        let outcome = repeat_on_schedule(&schedule, &cancel, &pause, move || {
            let c = count_clone.clone();
            async move { c.fetch_add(1, Ordering::SeqCst) }
        })
        .await
        .expect("recurs(2) should produce steps");

        assert_eq!(count.load(Ordering::SeqCst), 3);
        assert_eq!(outcome.attempts, 3);
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn repeat_on_never_returns_none() {
        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let schedule = Schedule::never();
        let outcome = repeat_on_schedule(&schedule, &cancel, &pause, || async { 42 }).await;
        assert!(outcome.is_none(), "never() should produce None");
    }

    #[tokio::test]
    async fn retry_on_schedule_succeeds_after_failures() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let schedule = Schedule::recurs(5);
        let outcome = retry_on_schedule(&schedule, &cancel, &pause, move || {
            let a = attempt_clone.clone();
            async move {
                let n = a.fetch_add(1, Ordering::SeqCst);
                if n < 2 {
                    Err("not yet")
                } else {
                    Ok(42)
                }
            }
        })
        .await
        .expect("recurs(5) should produce steps");

        assert_eq!(outcome.value, Ok(42));
        assert_eq!(outcome.attempts, 3);
        assert!(!outcome.cancelled);
    }

    #[tokio::test]
    async fn retry_on_schedule_when_respects_predicate() {
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let cancel = CancellationToken::new();
        let pause = PauseToken::new();
        let attempt = Arc::new(AtomicU32::new(0));
        let attempt_clone = attempt.clone();

        let schedule = Schedule::recurs(10);
        let outcome = retry_on_schedule_when(
            &schedule,
            &cancel,
            &pause,
            move || {
                let a = attempt_clone.clone();
                async move {
                    let n = a.fetch_add(1, Ordering::SeqCst);
                    Err::<i32, String>(format!("fail {n}"))
                }
            },
            |e| e != "fail 2", // stop retrying at "fail 2"
        )
        .await
        .expect("recurs(10) should produce steps");

        assert!(outcome.value.is_err());
        assert_eq!(outcome.attempts, 3);
    }

    // ── New law tests ──

    #[test]
    fn never_always_done() {
        let s = Schedule::never();
        for k in 0..10 {
            assert_eq!(s.step(k, Duration::ZERO), Decision::Done);
        }
    }

    #[test]
    fn jittered_is_deterministic_per_instance() {
        let s = Schedule::spaced(Duration::from_millis(200)).jittered();
        let d1 = s.step(0, Duration::ZERO);
        let d2 = s.step(0, Duration::ZERO);
        assert_eq!(d1, d2, "same schedule instance should produce same result");
    }

    #[test]
    fn schedule_clone_works() {
        let s = Schedule::spaced(Duration::from_millis(100))
            .jittered()
            .take(5);
        let s2 = s.clone();
        for k in 0..7 {
            assert_eq!(s.step(k, Duration::ZERO), s2.step(k, Duration::ZERO));
        }
    }

    #[test]
    fn schedule_debug_is_informative() {
        let s = Schedule::spaced(Duration::from_millis(100)).jittered();
        let dbg = format!("{s:?}");
        assert!(dbg.contains("spaced"), "debug should show variant: {dbg}");
        assert!(
            dbg.contains("jittered"),
            "debug should show jittered: {dbg}"
        );
    }
}
