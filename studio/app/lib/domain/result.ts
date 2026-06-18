/**
 * Result type for railway-oriented programming.
 *
 * Algebraic Laws:
 * - Result<A, E> forms a Bifunctor
 * - map(Ok(a), f) = Ok(f(a))
 * - map(Err(e), f) = Err(e)
 * - flatMap(Ok(a), f) = f(a)
 * - flatMap(Err(e), f) = Err(e)
 *
 * @module domain/result
 */

// ============================================================================
// Core Types
// ============================================================================

/**
 * Success case - contains the value.
 */
export interface Ok<A> {
  readonly _tag: "Ok";
  readonly value: A;
}

/**
 * Failure case - contains the error.
 */
export interface Err<E> {
  readonly _tag: "Err";
  readonly error: E;
}

/**
 * Result sum type - either Ok or Err.
 */
export type Result<A, E> = Ok<A> | Err<E>;

// ============================================================================
// Constructors
// ============================================================================

/**
 * Construct an Ok result.
 */
export const ok = <A>(value: A): Ok<A> => ({ _tag: "Ok", value });

/**
 * Construct an Err result.
 */
export const err = <E>(error: E): Err<E> => ({ _tag: "Err", error });

/**
 * Lift a nullable value into Result.
 */
export const fromNullable = <A, E>(value: A | null | undefined, onNull: () => E): Result<A, E> =>
  value != null ? ok(value) : err(onNull());

/**
 * Lift a try/catch into Result.
 */
export const tryCatch = <A, E>(f: () => A, onError: (e: unknown) => E): Result<A, E> => {
  try {
    return ok(f());
  } catch (e) {
    return err(onError(e));
  }
};

/**
 * Lift a Promise into Result.
 */
export const tryCatchAsync = async <A, E>(
  f: () => Promise<A>,
  onError: (e: unknown) => E
): Promise<Result<A, E>> => {
  try {
    return ok(await f());
  } catch (e) {
    return err(onError(e));
  }
};

// ============================================================================
// Type Guards
// ============================================================================

/**
 * Check if result is Ok.
 */
export const isOk = <A, E>(result: Result<A, E>): result is Ok<A> => result._tag === "Ok";

/**
 * Check if result is Err.
 */
export const isErr = <A, E>(result: Result<A, E>): result is Err<E> => result._tag === "Err";

// ============================================================================
// Transformations (Functor, Bifunctor)
// ============================================================================

/**
 * Map over the success value.
 *
 * Law: map(Ok(a), f) = Ok(f(a))
 * Law: map(Err(e), f) = Err(e)
 */
export const map = <A, B, E>(result: Result<A, E>, f: (a: A) => B): Result<B, E> =>
  isOk(result) ? ok(f(result.value)) : result;

/**
 * Map over the error value.
 *
 * Law: mapError(Ok(a), f) = Ok(a)
 * Law: mapError(Err(e), f) = Err(f(e))
 */
export const mapError = <A, E, E2>(result: Result<A, E>, f: (e: E) => E2): Result<A, E2> =>
  isErr(result) ? err(f(result.error)) : result;

/**
 * Map over both channels (Bifunctor).
 */
export const bimap = <A, B, E, E2>(
  result: Result<A, E>,
  onOk: (a: A) => B,
  onErr: (e: E) => E2
): Result<B, E2> => (isOk(result) ? ok(onOk(result.value)) : err(onErr(result.error)));

// ============================================================================
// Chaining (Monad)
// ============================================================================

/**
 * Chain computations that may fail.
 *
 * Law: flatMap(Ok(a), f) = f(a)
 * Law: flatMap(Err(e), f) = Err(e)
 */
export const flatMap = <A, B, E>(result: Result<A, E>, f: (a: A) => Result<B, E>): Result<B, E> =>
  isOk(result) ? f(result.value) : result;

/**
 * Chain with error recovery.
 */
export const catchError = <A, E, E2>(
  result: Result<A, E>,
  f: (e: E) => Result<A, E2>
): Result<A, E2> => (isErr(result) ? f(result.error) : result);

// ============================================================================
// Folding / Pattern Matching
// ============================================================================

/**
 * Fold the result into a single value.
 */
export const fold = <A, E, B>(result: Result<A, E>, onErr: (e: E) => B, onOk: (a: A) => B): B =>
  isOk(result) ? onOk(result.value) : onErr(result.error);

/**
 * Pattern match on Result (object syntax, recommended).
 *
 * @example
 * match(result, {
 *   ok: (value) => `Success: ${value}`,
 *   err: (error) => `Error: ${error.message}`,
 * })
 */
export const match = <A, E, B>(
  result: Result<A, E>,
  handlers: { ok: (a: A) => B; err: (e: E) => B }
): B => (isOk(result) ? handlers.ok(result.value) : handlers.err(result.error));

/**
 * Get the value or a default.
 */
export const getOrElse = <A, E>(result: Result<A, E>, defaultValue: A): A =>
  isOk(result) ? result.value : defaultValue;

/**
 * Get the value or compute a default.
 */
export const getOrElseW = <A, E, B>(result: Result<A, E>, onErr: (e: E) => B): A | B =>
  isOk(result) ? result.value : onErr(result.error);

// ============================================================================
// Utilities
// ============================================================================

/**
 * Combine multiple Results (all must succeed).
 */
export const all = <A, E>(results: Result<A, E>[]): Result<A[], E> => {
  const values: A[] = [];
  for (const result of results) {
    if (isErr(result)) return result;
    values.push(result.value);
  }
  return ok(values);
};

/**
 * Partition results into successes and failures.
 */
export const partition = <A, E>(results: Result<A, E>[]): { ok: A[]; err: E[] } => {
  const okValues: A[] = [];
  const errValues: E[] = [];
  for (const result of results) {
    if (isOk(result)) {
      okValues.push(result.value);
    } else {
      errValues.push(result.error);
    }
  }
  return { ok: okValues, err: errValues };
};
