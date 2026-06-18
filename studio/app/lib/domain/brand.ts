/**
 * Branded types utility for nominal typing.
 *
 * Branded types prevent mixing structurally-identical primitives
 * at compile time while remaining zero-cost at runtime.
 *
 * @module domain/brand
 */

declare const __brand: unique symbol;

export type Brand<K extends string, T> = T & { readonly [__brand]: K };
