/**
 * Branded types utility for nominal typing.
 *
 * Branded types prevent mixing structurally-identical primitives
 * (e.g., ThreadId vs WorkspaceId) at compile time.
 *
 * @module domain/brand
 */

declare const __brand: unique symbol;

/**
 * Branded type — attaches a phantom tag K to base type T.
 *
 * At runtime this is just T, but at compile time it's a distinct type.
 */
export type Brand<K extends string, T> = T & { readonly [__brand]: K };
