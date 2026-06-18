export interface EvalScorerLike {
  readonly scorerName: string;
  readonly score: number;
}

export interface EvalMatrixSampleLike<TScorer extends EvalScorerLike = EvalScorerLike> {
  readonly passed: boolean;
  readonly scores: readonly TScorer[];
}

export type ParsedEvalScorerDetailsLike =
  | {
      readonly kind: "trajectory";
      readonly fBeta: {
        readonly fScore: number;
        readonly precision: number;
        readonly recall: number;
      };
    }
  | {
      readonly kind: "actionMatch";
      readonly actionMatch: {
        readonly summary: {
          readonly total: number;
          readonly exact: number;
        };
      };
    }
  | {
      readonly kind: "fusedExecutor";
      readonly evaluation: {
        readonly weightedScore: number;
        readonly pass: boolean;
      };
    }
  | {
      readonly kind: "generic";
      readonly scorerName: string;
      readonly score: number;
    };

export function getEvalScoreIntensityColor<TScorer extends EvalScorerLike>(
  sample: EvalMatrixSampleLike<TScorer> | undefined,
  options: {
    readonly passThreshold: number;
    readonly queuedColor: string;
    readonly extractSampleScore: (scores: readonly TScorer[]) => number;
  }
): string {
  const { passThreshold, queuedColor, extractSampleScore } = options;

  if (!sample) return queuedColor;

  const score = extractSampleScore(sample.scores);

  if (sample.passed) {
    const t = passThreshold < 1 ? Math.min(1, (score - passThreshold) / (1 - passThreshold)) : 1;
    const lightness = 55 - t * 20;
    const saturation = 60 + t * 15;
    return `hsl(145, ${saturation}%, ${lightness}%)`;
  }

  const t = passThreshold > 0 ? Math.min(1, 1 - score / passThreshold) : 1;
  const lightness = 55 - t * 15;
  const saturation = 60 + t * 15;
  return `hsl(0, ${saturation}%, ${lightness}%)`;
}

export function formatEvalScoreBreakdown<TScorer extends EvalScorerLike>(
  scores: readonly TScorer[],
  options: {
    readonly parseScorerDetails?: (scorer: TScorer) => ParsedEvalScorerDetailsLike;
  } = {}
): string {
  const scorer = scores[0];
  if (!scorer) return "N/A";

  if (options.parseScorerDetails) {
    const parsed = options.parseScorerDetails(scorer);
    switch (parsed.kind) {
      case "trajectory":
        return `F-β: ${parsed.fBeta.fScore.toFixed(3)} (P=${parsed.fBeta.precision.toFixed(2)}, R=${parsed.fBeta.recall.toFixed(2)})`;
      case "actionMatch": {
        const { summary } = parsed.actionMatch;
        const score = summary.total === 0 ? 1 : summary.exact / summary.total;
        return `Actions: ${(score * 100).toFixed(0)}% (${summary.exact}/${summary.total} exact)`;
      }
      case "fusedExecutor":
        return `Fused: ${parsed.evaluation.weightedScore.toFixed(3)} (${parsed.evaluation.pass ? "PASS" : "FAIL"})`;
      case "generic":
        return `${parsed.scorerName || "score"}: ${parsed.score.toFixed(3)}`;
    }
  }

  return `${scorer.scorerName || "score"}: ${scorer.score.toFixed(3)}`;
}
