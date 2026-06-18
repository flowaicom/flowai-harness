import { describe, expect, test } from "bun:test";
import { selectModelSettings, useAgentConfig } from "./settings-store";

describe("settings store selectors", () => {
  test("model settings selector returns a stable reference for unchanged inputs", () => {
    const state = useAgentConfig.getState();
    const first = selectModelSettings(state);
    const second = selectModelSettings(state);

    expect(first).toBe(second);
  });
});
