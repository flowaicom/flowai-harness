export interface ConnectRuntimeErrorLike {
  readonly message: string;
}

export type ConnectRuntimeResult<
  TValue,
  TError extends ConnectRuntimeErrorLike = ConnectRuntimeErrorLike,
> =
  | {
      readonly _tag: "Ok";
      readonly value: TValue;
    }
  | {
      readonly _tag: "Err";
      readonly error: TError;
    };

export function isConnectRuntimeOk<
  TValue,
  TError extends ConnectRuntimeErrorLike = ConnectRuntimeErrorLike,
>(
  result: ConnectRuntimeResult<TValue, TError>
): result is { readonly _tag: "Ok"; readonly value: TValue } {
  return result._tag === "Ok";
}
