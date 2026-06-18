import type { SharedTestCaseStatus } from "./domain";

export type TestSidebarFilterValue = SharedTestCaseStatus | "all";

export interface TestSidebarCaseLike {
  readonly id: string;
  readonly input: string;
  readonly status: SharedTestCaseStatus;
  readonly tags?: readonly string[] | null;
  readonly expectedTrajectory?: readonly string[] | null;
  readonly name?: string | null;
}

export function extractTestCaseLevel(id: string): string | null {
  const match = id.match(/(L\d+(?:-\d+)?)/i);
  return match ? match[1].toUpperCase() : null;
}

export function collectTestSidebarTags<T extends TestSidebarCaseLike>(
  testCases: readonly T[]
): string[] {
  const tagSet = new Set<string>();
  for (const testCase of testCases) {
    for (const tag of testCase.tags ?? []) {
      tagSet.add(tag);
    }
  }
  return [...tagSet].sort();
}

export function countTestCasesByStatus<T extends Pick<TestSidebarCaseLike, "status">>(
  testCases: readonly T[]
): Record<TestSidebarFilterValue, number> {
  const counts: Record<TestSidebarFilterValue, number> = {
    all: 0,
    active: 0,
    draft: 0,
    archived: 0,
  };

  for (const testCase of testCases) {
    counts.all++;
    counts[testCase.status]++;
  }

  return counts;
}

export function filterTestCasesByQuery<T extends TestSidebarCaseLike>(
  testCases: readonly T[],
  query: string
): T[] {
  const normalizedQuery = query.trim().toLowerCase();
  if (!normalizedQuery) {
    return [...testCases];
  }

  return testCases.filter((testCase) =>
    [
      testCase.input,
      testCase.name ?? "",
      ...(testCase.tags ?? []),
      ...(testCase.expectedTrajectory ?? []),
    ].some((value) => value.toLowerCase().includes(normalizedQuery))
  );
}
