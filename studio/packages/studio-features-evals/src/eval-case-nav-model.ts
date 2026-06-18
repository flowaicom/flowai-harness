export function getEvalCaseSampleNavIndex(
  key: string,
  sampleIndex: number,
  sampleCount: number
): number | null {
  if (key === "ArrowLeft" && sampleIndex > 0) {
    return sampleIndex - 1;
  }
  if (key === "ArrowRight" && sampleIndex < sampleCount - 1) {
    return sampleIndex + 1;
  }
  return null;
}

export function getAdjacentEvalCaseId(
  key: string,
  testCaseIds: readonly string[],
  testCaseId: string
): string | null {
  const currentIdx = testCaseIds.indexOf(testCaseId);

  if ((key === "j" || key === "J") && currentIdx < testCaseIds.length - 1) {
    return testCaseIds[currentIdx + 1] ?? null;
  }

  if ((key === "k" || key === "K") && currentIdx > 0) {
    return testCaseIds[currentIdx - 1] ?? null;
  }

  return null;
}

export function buildEvalCaseRoute(evalId: string, testCaseId: string, sampleIndex = 0): string {
  return `/evals/${evalId}/cases/${testCaseId}?sample=${sampleIndex}`;
}
