import { describe, expect, test } from "bun:test";
import * as fc from "fast-check";

import { arbErr, arbKleisli, arbOk, arbResult } from "~/lib/test-utils/arbitraries";
import {
  all,
  bimap,
  catchError,
  err,
  flatMap,
  fold,
  fromNullable,
  getOrElse,
  getOrElseW,
  isErr,
  isOk,
  map,
  mapError,
  match as matchResult,
  ok,
  partition,
  type Result,
  tryCatch,
} from "./result";

const id = <T>(x: T): T => x;

// ============================================================================
// Functor Laws
// ============================================================================

describe("Functor laws", () => {
  test("identity: map(ok(a), id) = ok(a)", () => {
    expect(map(ok(42), id)).toEqual(ok(42));
  });

  test("identity on Err: map(err(e), id) = err(e)", () => {
    expect(map(err("fail"), id)).toEqual(err("fail"));
  });

  test("composition: map(map(r, f), g) = map(r, x => g(f(x)))", () => {
    const f = (x: number) => x + 1;
    const g = (x: number) => x * 2;
    const r = ok(5);
    expect(map(map(r, f), g)).toEqual(map(r, (x) => g(f(x))));
  });

  test("composition on Err: both sides produce same Err", () => {
    const f = (x: number) => x + 1;
    const g = (x: number) => x * 2;
    const r = err("fail");
    expect(map(map(r, f), g)).toEqual(map(r, (x: number) => g(f(x))));
  });
});

// ============================================================================
// Bifunctor Laws
// ============================================================================

describe("Bifunctor laws", () => {
  test("identity: bimap(r, id, id) = r", () => {
    expect(bimap(ok(1), id, id)).toEqual(ok(1));
    expect(bimap(err("e"), id, id)).toEqual(err("e"));
  });
});

// ============================================================================
// Monad Laws
// ============================================================================

describe("Monad laws", () => {
  test("left identity: flatMap(ok(a), f) = f(a)", () => {
    const f = (x: number) => ok(x + 1);
    expect(flatMap(ok(42), f)).toEqual(f(42));
  });

  test("right identity: flatMap(m, ok) = m", () => {
    const m = ok(42);
    expect(flatMap(m, ok)).toEqual(m);
  });

  test("right identity on Err: flatMap(err, ok) = err", () => {
    const m = err("fail");
    expect(flatMap(m, ok)).toEqual(m);
  });

  test("associativity: flatMap(flatMap(m, f), g) = flatMap(m, x => flatMap(f(x), g))", () => {
    const f = (x: number) => ok(x + 1);
    const g = (x: number) => ok(x * 2);
    const m = ok(5);
    expect(flatMap(flatMap(m, f), g)).toEqual(flatMap(m, (x) => flatMap(f(x), g)));
  });

  test("Err short-circuits: flatMap(err(e), f) = err(e)", () => {
    const f = (x: number) => ok(x + 1);
    expect(flatMap(err("fail"), f)).toEqual(err("fail"));
  });
});

// ============================================================================
// mapError
// ============================================================================

describe("mapError laws", () => {
  test("identity: mapError(err(e), id) = err(e)", () => {
    expect(mapError(err("fail"), id)).toEqual(err("fail"));
  });

  test("Ok pass-through: mapError(ok(a), f) = ok(a)", () => {
    expect(mapError(ok(42), () => "mapped")).toEqual(ok(42));
  });
});

// ============================================================================
// catchError
// ============================================================================

describe("catchError", () => {
  test("recovery: catchError(err(e), f) = f(e)", () => {
    const f = (e: string) => ok(e.length);
    expect(catchError(err("fail"), f)).toEqual(f("fail"));
  });

  test("Ok pass-through: catchError(ok(a), f) = ok(a)", () => {
    expect(catchError(ok(42), () => ok(0))).toEqual(ok(42));
  });
});

// ============================================================================
// fold / match
// ============================================================================

describe("fold", () => {
  test("Ok branch", () => {
    expect(
      fold(
        ok(42),
        () => "err",
        (a) => `ok:${a}`
      )
    ).toBe("ok:42");
  });

  test("Err branch", () => {
    expect(
      fold(
        err("fail"),
        (e) => `err:${e}`,
        () => "ok"
      )
    ).toBe("err:fail");
  });
});

describe("match", () => {
  test("Ok branch", () => {
    expect(matchResult(ok(42), { ok: (a) => a + 1, err: () => 0 })).toBe(43);
  });

  test("Err branch", () => {
    expect(matchResult(err("e"), { ok: () => 0, err: (e) => e.length })).toBe(1);
  });
});

// ============================================================================
// getOrElse
// ============================================================================

describe("getOrElse", () => {
  test("Ok returns value", () => {
    expect(getOrElse(ok(42), 0)).toBe(42);
  });

  test("Err returns default", () => {
    expect(getOrElse(err("fail"), 0)).toBe(0);
  });
});

describe("getOrElseW", () => {
  test("Ok returns value", () => {
    expect(getOrElseW(ok(42), () => "default")).toBe(42);
  });

  test("Err computes default", () => {
    expect(getOrElseW(err("fail"), (e) => e.length)).toBe(4);
  });
});

// ============================================================================
// all
// ============================================================================

describe("all", () => {
  test("all Ok produces Ok of array", () => {
    expect(all([ok(1), ok(2), ok(3)])).toEqual(ok([1, 2, 3]));
  });

  test("any Err produces first Err", () => {
    expect(all([ok(1), err("fail"), ok(3)])).toEqual(err("fail"));
  });

  test("empty produces Ok([])", () => {
    expect(all([])).toEqual(ok([]));
  });
});

// ============================================================================
// partition
// ============================================================================

describe("partition", () => {
  test("separates Ok and Err", () => {
    const results = [ok(1), err("a"), ok(2), err("b")];
    expect(partition(results)).toEqual({ ok: [1, 2], err: ["a", "b"] });
  });

  test("empty input", () => {
    expect(partition([])).toEqual({ ok: [], err: [] });
  });
});

// ============================================================================
// constructors
// ============================================================================

describe("constructors", () => {
  test("fromNullable: non-null -> Ok", () => {
    expect(fromNullable(42, () => "err")).toEqual(ok(42));
  });

  test("fromNullable: null -> Err", () => {
    expect(fromNullable(null, () => "was null")).toEqual(err("was null"));
  });

  test("fromNullable: undefined -> Err", () => {
    expect(fromNullable(undefined, () => "undef")).toEqual(err("undef"));
  });

  test("tryCatch: success -> Ok", () => {
    expect(
      tryCatch(
        () => 42,
        () => "err"
      )
    ).toEqual(ok(42));
  });

  test("tryCatch: throw -> Err", () => {
    const r = tryCatch(
      () => {
        throw new Error("boom");
      },
      (e) => (e as Error).message
    );
    expect(r).toEqual(err("boom"));
  });
});

// ============================================================================
// type guards
// ============================================================================

describe("type guards", () => {
  test("isOk on Ok", () => expect(isOk(ok(1))).toBe(true));
  test("isOk on Err", () => expect(isOk(err("e"))).toBe(false));
  test("isErr on Err", () => expect(isErr(err("e"))).toBe(true));
  test("isErr on Ok", () => expect(isErr(ok(1))).toBe(false));
});

// ============================================================================
// Property-Based Tests (Interpreter Layer)
//
// Each test evolves one or more concrete examples into a universally
// quantified property. fc.property() describes the law (program-as-value);
// fc.assert() interprets it.
// ============================================================================

// -- Functor Laws --

describe("Functor laws (property-based)", () => {
  test("identity: map(r, id) = r for all Result", () => {
    fc.assert(
      fc.property(arbResult, (r) => {
        expect(map(r, id)).toEqual(r);
      })
    );
  });

  test("composition: map(map(r, f), g) = map(r, x => g(f(x)))", () => {
    fc.assert(
      fc.property(arbResult, fc.func(fc.integer()), fc.func(fc.integer()), (r, f, g) => {
        expect(map(map(r, f), g)).toEqual(map(r, (x) => g(f(x))));
      })
    );
  });
});

// -- Bifunctor Laws --

describe("Bifunctor laws (property-based)", () => {
  test("identity: bimap(r, id, id) = r for all Result", () => {
    fc.assert(
      fc.property(arbResult, (r) => {
        expect(bimap(r, id, id)).toEqual(r);
      })
    );
  });
});

// -- Monad Laws --

describe("Monad laws (property-based)", () => {
  test("left identity: flatMap(ok(a), f) = f(a)", () => {
    fc.assert(
      fc.property(fc.integer(), arbKleisli, (a, f) => {
        expect(flatMap(ok(a), f)).toEqual(f(a));
      })
    );
  });

  test("right identity: flatMap(m, ok) = m", () => {
    fc.assert(
      fc.property(arbResult, (m) => {
        expect(flatMap(m, ok)).toEqual(m);
      })
    );
  });

  test("associativity: flatMap(flatMap(m, f), g) = flatMap(m, x => flatMap(f(x), g))", () => {
    fc.assert(
      fc.property(arbResult, arbKleisli, arbKleisli, (m, f, g) => {
        const left = flatMap(flatMap(m, f), g);
        const right = flatMap(m, (x) => flatMap(f(x), g));
        expect(left).toEqual(right);
      })
    );
  });

  test("Err short-circuit: flatMap(err(e), f) = err(e)", () => {
    fc.assert(
      fc.property(fc.string(), arbKleisli, (e, f) => {
        expect(flatMap(err(e), f)).toEqual(err(e));
      })
    );
  });
});

// -- mapError --

describe("mapError (property-based)", () => {
  test("identity: mapError(r, id) = r", () => {
    fc.assert(
      fc.property(arbResult, (r) => {
        expect(mapError(r, id)).toEqual(r);
      })
    );
  });
});

// -- Type Guard Complement --

describe("type guards (property-based)", () => {
  test("isOk and isErr are complementary", () => {
    fc.assert(
      fc.property(arbResult, (r) => {
        expect(isOk(r)).toBe(!isErr(r));
      })
    );
  });
});

// -- fold / match --

describe("fold (property-based)", () => {
  test("dispatches to the correct branch with the correct value", () => {
    fc.assert(
      fc.property(arbResult, (r) => {
        const result = fold<
          number,
          string,
          { branch: "err"; val: string } | { branch: "ok"; val: number }
        >(
          r,
          (e) => ({ branch: "err" as const, val: e }),
          (a) => ({ branch: "ok" as const, val: a })
        );
        if (isOk(r)) {
          expect(result).toEqual({ branch: "ok", val: r.value });
        } else {
          expect(result).toEqual({ branch: "err", val: r.error });
        }
      })
    );
  });
});

// -- getOrElse --

describe("getOrElse (property-based)", () => {
  test("Ok returns value, Err returns default", () => {
    fc.assert(
      fc.property(arbResult, fc.integer(), (r, defaultVal) => {
        const result = getOrElse(r, defaultVal);
        if (isOk(r)) {
          expect(result).toBe(r.value);
        } else {
          expect(result).toBe(defaultVal);
        }
      })
    );
  });
});

// -- all --

describe("all (property-based)", () => {
  test("all-Ok array roundtrip: all(vs.map(ok)).value = vs", () => {
    fc.assert(
      fc.property(fc.array(fc.integer()), (values) => {
        const combined = all(values.map(ok));
        expect(isOk(combined)).toBe(true);
        if (isOk(combined)) {
          expect(combined.value).toEqual(values);
        }
      })
    );
  });

  test("returns the first Err in order", () => {
    fc.assert(
      fc.property(fc.array(arbResult, { minLength: 1 }), (results) => {
        const combined = all(results);
        const firstErrIdx = results.findIndex(isErr);
        if (firstErrIdx === -1) {
          expect(isOk(combined)).toBe(true);
        } else {
          expect(combined).toEqual(results[firstErrIdx]);
        }
      })
    );
  });
});

// -- partition --

describe("partition (property-based)", () => {
  test("length conservation: ok.length + err.length = input.length", () => {
    fc.assert(
      fc.property(fc.array(arbResult), (results) => {
        const { ok: oks, err: errs } = partition(results);
        expect(oks.length + errs.length).toBe(results.length);
      })
    );
  });

  test("preserves values in order", () => {
    fc.assert(
      fc.property(fc.array(arbResult), (results) => {
        const { ok: oks, err: errs } = partition(results);
        const expectedOks = results.filter(isOk).map((r) => r.value);
        const expectedErrs = results.filter(isErr).map((r) => r.error);
        expect(oks).toEqual(expectedOks);
        expect(errs).toEqual(expectedErrs);
      })
    );
  });
});

// -- fromNullable --

describe("fromNullable (property-based)", () => {
  test("Ok iff value is non-null", () => {
    const arbNullable = fc.oneof(
      fc.integer().map((n) => n as number | null | undefined),
      fc.constant(null as number | null | undefined),
      fc.constant(undefined as number | null | undefined)
    );
    fc.assert(
      fc.property(arbNullable, (value) => {
        const result = fromNullable(value, () => "was null");
        if (value != null) {
          expect(result).toEqual(ok(value));
        } else {
          expect(result).toEqual(err("was null"));
        }
      })
    );
  });
});

// -- tryCatch --

describe("tryCatch (property-based)", () => {
  test("non-throwing wraps as Ok", () => {
    fc.assert(
      fc.property(fc.integer(), (a) => {
        const result = tryCatch(
          () => a,
          (e) => String(e)
        );
        expect(result).toEqual(ok(a));
      })
    );
  });

  test("throwing wraps as Err", () => {
    fc.assert(
      fc.property(fc.string(), (msg) => {
        const result = tryCatch(
          () => {
            throw new Error(msg);
          },
          (e) => (e as Error).message
        );
        expect(isErr(result)).toBe(true);
        if (isErr(result)) {
          expect(result.error).toBe(msg);
        }
      })
    );
  });
});
