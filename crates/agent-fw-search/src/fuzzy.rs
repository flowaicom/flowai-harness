//! Point-based fuzzy matching with token-boundary awareness.
//!
//! # Scoring Model
//!
//! Every matching signal contributes discrete, non-overlapping points:
//!
//! | Signal | Points | Phase | Cost |
//! |--------|--------|-------|------|
//! | Exact normalized | 100 | 1 | O(1) |
//! | Token containment | 80–98 | 1 | O(tokens) |
//! | Token edit-dist ≤ 1 | 60 | 1 | O(tokens) |
//! | Jaccard n-gram | 10–40 | 1b | O(n) |
//! | Vector similarity | 25–50 | 2 | ~50ms |
//!
//! Non-overlapping tiers guarantee monotonicity: Exact always beats Token,
//! Token always beats edit distance, etc. No threshold tuning needed.
//!
//! # Laws
//!
//! 1. **Identity**: `score(x, x) = Exact(100)`
//! 2. **Normalization**: `score(normalize(x), x) = Exact(100)`
//! 3. **Numeric exactness**: `"2"` never matches `"20"` in token tiers
//! 4. **Precision monotonicity**: `score("Brand 2", "Brand 2") > score("Brand 2", "Brand 20")`
//! 5. **Tier separation**: max(Tier N+1) < min(Tier N)

use std::collections::HashSet;

// =============================================================================
// VectorMatch — pre-computed vector similarity for the pure pipeline
// =============================================================================

/// Pre-computed vector similarity result for the pure resolve_value pipeline.
/// Produced by async callers (vector_search_for_column), consumed by sync scoring.
#[derive(Clone, Debug)]
pub struct VectorMatch {
    /// The db_value this hit maps to.
    pub value: String,
    /// Cosine similarity [0.0, 1.0].
    pub cosine: f64,
}

// =============================================================================
// MatchScore — transparent point-based scoring
// =============================================================================

/// Point-based match score with signal breakdown.
///
/// Each signal contributes discrete points. The total determines acceptance.
/// The breakdown is preserved for debugging, logging, and UI display.
#[derive(Clone, Debug)]
pub struct MatchScore {
    /// Total points (sum of all signal contributions).
    pub points: u32,
    /// Which signals contributed to this score.
    pub signals: Vec<MatchSignal>,
    /// The resolved database value.
    pub value: String,
}

impl MatchScore {
    fn new(value: String, signals: Vec<MatchSignal>) -> Self {
        let points = signals.iter().map(|s| s.points()).sum();
        Self {
            points,
            signals,
            value,
        }
    }

    /// Project to a wire-format score summary (value + points).
    pub fn summarize(&self) -> ScoredMatch {
        ScoredMatch {
            value: self.value.clone(),
            score: self.points,
        }
    }
}

/// A match projected to value + score for LLM-facing output.
#[derive(Clone, Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoredMatch {
    /// The matched database value.
    pub value: String,
    /// Match score (0–100 points).
    pub score: u32,
}

/// A matching signal with its point contribution.
#[derive(Clone, Debug)]
pub enum MatchSignal {
    /// Exact match after normalization (case-fold, separator-fold).
    Exact,
    /// All search tokens found as whole tokens in candidate.
    TokenContainment {
        matched_tokens: u32,
        total_candidate_tokens: u32,
    },
    /// Token match within edit distance 1 (numeric tokens still exact).
    EditDistance { original: String, matched: String },
    /// Jaccard n-gram containment (character-level bigrams).
    JaccardNgram { similarity: f64 },
    /// Vector similarity via embeddings.
    VectorSimilarity { cosine: f64 },
    /// Search term matches a complete segment in a structured compound name.
    SegmentExact { segment_index: u32 },
}

impl MatchSignal {
    /// Human-readable label for LLM-facing output.
    pub fn label(&self) -> String {
        match self {
            Self::Exact => "exact".to_string(),
            Self::TokenContainment {
                matched_tokens,
                total_candidate_tokens,
            } => format!("token({}/{})", matched_tokens, total_candidate_tokens),
            Self::EditDistance {
                original, matched, ..
            } => format!("edit({}→{})", original, matched),
            Self::JaccardNgram { similarity } => format!("jaccard({:.0}%)", similarity * 100.0),
            Self::VectorSimilarity { cosine } => format!("vector({:.0}%)", cosine * 100.0),
            Self::SegmentExact { segment_index } => format!("segment({})", segment_index),
        }
    }

    /// Point contribution for this signal.
    pub fn points(&self) -> u32 {
        match self {
            Self::Exact => 100,
            Self::TokenContainment {
                matched_tokens,
                total_candidate_tokens,
            } => {
                let coverage = if *total_candidate_tokens > 0 {
                    *matched_tokens as f64 / *total_candidate_tokens as f64
                } else {
                    0.0
                };
                80 + (coverage.clamp(0.0, 1.0) * 18.0) as u32
            }
            Self::EditDistance { .. } => 60,
            Self::JaccardNgram { similarity } => (similarity.clamp(0.0, 1.0) * 40.0) as u32,
            Self::VectorSimilarity { cosine } => (cosine.clamp(0.0, 1.0) * 50.0) as u32,
            Self::SegmentExact { .. } => 95,
        }
    }
}

/// Resolution strategy — determines acceptance threshold and result mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveStrategy {
    /// For categorical columns: high threshold.
    Categorical,
    /// For display_name / free text: lower threshold.
    DisplayName,
}

/// Configurable thresholds per-column.
pub struct ResolveConfig {
    /// Map column name to resolution strategy.
    pub strategy_for_column: Box<dyn Fn(&str) -> ResolveStrategy + Send + Sync>,
    /// Minimum points for categorical matches.
    pub categorical_threshold: u32,
    /// Minimum points for display_name matches.
    pub display_name_threshold: u32,
}

impl ResolveConfig {
    /// Default configuration: display_name → DisplayName, everything else → Categorical.
    pub fn default_config() -> Self {
        Self {
            strategy_for_column: Box::new(|column| {
                if column == "display_name" {
                    ResolveStrategy::DisplayName
                } else {
                    ResolveStrategy::Categorical
                }
            }),
            categorical_threshold: 60,
            display_name_threshold: 40,
        }
    }

    /// Get the threshold for a given strategy.
    pub fn threshold(&self, strategy: ResolveStrategy) -> u32 {
        match strategy {
            ResolveStrategy::Categorical => self.categorical_threshold,
            ResolveStrategy::DisplayName => self.display_name_threshold,
        }
    }
}

impl ResolveStrategy {
    /// Determine the resolution strategy for a given column (default mapping).
    pub fn for_column(column: &str) -> Self {
        if column == "display_name" {
            Self::DisplayName
        } else {
            Self::Categorical
        }
    }

    /// Minimum points required for a match to be accepted (default thresholds).
    pub fn threshold(self) -> u32 {
        match self {
            Self::Categorical => 60,
            Self::DisplayName => 40,
        }
    }
}

// =============================================================================
// Normalization & tokenization
// =============================================================================

/// Normalize a string: lowercase, all separators→space, collapse whitespace.
///
/// Single-pass implementation: 1 allocation (the output String).
pub fn normalize(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut prev_was_space = true;
    for c in text.chars() {
        if c == '_' || c == '-' || c.is_whitespace() {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            for lc in c.to_lowercase() {
                result.push(lc);
            }
            prev_was_space = false;
        }
    }
    if result.ends_with(' ') {
        result.pop();
    }
    result
}

/// Split a normalized string into tokens.
fn tokenize(text: &str) -> Vec<String> {
    text.split_whitespace().map(String::from).collect()
}

/// Check if a token is purely numeric (digits only).
fn is_numeric(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_ascii_digit())
}

// =============================================================================
// Segment-level matching for compound names
// =============================================================================

fn segment_exact_match(search_norm: &str, candidate_original: &str) -> Option<u32> {
    if !candidate_original.contains(" - ") {
        return None;
    }
    candidate_original
        .split(" - ")
        .enumerate()
        .find(|(_, seg)| normalize(seg) == search_norm)
        .map(|(i, _)| i as u32)
}

/// Check if a match has a SegmentExact signal.
pub fn has_segment_signal(score: &MatchScore) -> bool {
    score
        .signals
        .iter()
        .any(|s| matches!(s, MatchSignal::SegmentExact { .. }))
}

// =============================================================================
// Resolution phase predicates
// =============================================================================

/// Phase 1 scoring was too weak — vector fallback should be attempted.
pub fn needs_vector_fallback(matches: &[MatchScore]) -> bool {
    matches.first().map_or(true, |m| m.points < 60)
}

/// Extract matched values from fuzzy scores according to strategy semantics.
///
/// - `Categorical`: single best match (first element = highest score).
///   For low-cardinality columns (product_type, brand).
/// - `DisplayName`: all matches above threshold.
///   For high-cardinality columns (display_name) where multiple entities match.
///
/// Input must be sorted by score descending (as returned by `resolve_value_amortized`).
///
/// # Laws
///
/// - **L1 (Empty preservation)**: `collect_matched_values([], _) = []`
/// - **L2 (Categorical bound)**: `|collect_matched_values(ms, Categorical)| <= 1`
/// - **L3 (DisplayName totality)**: When `ms` non-empty, `|collect_matched_values(ms, DisplayName)| == |ms|`
/// - **L4 (Best inclusion)**: When `ms` non-empty, `ms[0].value` is in the result
pub fn collect_matched_values(matches: Vec<MatchScore>, strategy: ResolveStrategy) -> Vec<String> {
    if matches.is_empty() {
        return Vec::new();
    }
    match strategy {
        ResolveStrategy::Categorical => {
            // Single best match
            vec![matches.into_iter().next().unwrap().value]
        }
        ResolveStrategy::DisplayName => {
            // All matches
            matches.into_iter().map(|m| m.value).collect()
        }
    }
}

/// Segment-exact matches are present; scattered token matches should be filtered.
pub fn has_scattered_token_noise(query: &PreparedQuery, matches: &[MatchScore]) -> bool {
    query.tokens.len() >= 2 && matches.iter().any(has_segment_signal)
}

/// Remove scattered token matches when segment-exact results dominate.
pub fn filter_scattered_matches(matches: &mut Vec<MatchScore>) {
    matches.retain(|m| m.points >= 90);
}

// =============================================================================
// Phase 1: Token-boundary-aware matching
// =============================================================================

fn token_containment(search_tokens: &[String], candidate_tokens: &[String]) -> Option<MatchSignal> {
    if search_tokens.is_empty() || candidate_tokens.is_empty() {
        return None;
    }

    let all_match = if candidate_tokens.len() > 4 {
        let set: HashSet<&str> = candidate_tokens.iter().map(|s| s.as_str()).collect();
        search_tokens.iter().all(|st| set.contains(st.as_str()))
    } else {
        search_tokens
            .iter()
            .all(|st| candidate_tokens.iter().any(|ct| ct == st))
    };

    if all_match {
        Some(MatchSignal::TokenContainment {
            matched_tokens: search_tokens.len() as u32,
            total_candidate_tokens: candidate_tokens.len() as u32,
        })
    } else {
        None
    }
}

fn token_edit_match(search_tokens: &[String], candidate_tokens: &[String]) -> Option<MatchSignal> {
    if search_tokens.is_empty() || candidate_tokens.is_empty() {
        return None;
    }

    let mut first_edit: Option<(String, String)> = None;

    for st in search_tokens {
        let found = candidate_tokens.iter().any(|ct| {
            if is_numeric(st) || is_numeric(ct) {
                st == ct
            } else if st.len() >= 3 && ct.len() >= 3 {
                if st == ct {
                    true
                } else if edit_distance_at_most_one(st, ct) {
                    if first_edit.is_none() {
                        first_edit = Some((st.clone(), ct.clone()));
                    }
                    true
                } else {
                    false
                }
            } else {
                st == ct
            }
        });
        if !found {
            return None;
        }
    }

    first_edit.map(|(original, matched)| MatchSignal::EditDistance { original, matched })
}

/// O(n) check: is Damerau-Levenshtein distance between `a` and `b` at most 1?
pub fn edit_distance_at_most_one(a: &str, b: &str) -> bool {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let len_diff = (a_chars.len() as isize - b_chars.len() as isize).unsigned_abs();

    if len_diff > 1 {
        return false;
    }

    if a_chars.len() == b_chars.len() {
        let diff_positions: Vec<usize> = a_chars
            .iter()
            .zip(&b_chars)
            .enumerate()
            .filter(|(_, (ca, cb))| ca != cb)
            .map(|(i, _)| i)
            .collect();

        match diff_positions.len() {
            0 => true,
            1 => true,
            2 => {
                let (i, j) = (diff_positions[0], diff_positions[1]);
                j == i + 1 && a_chars[i] == b_chars[j] && a_chars[j] == b_chars[i]
            }
            _ => false,
        }
    } else {
        let (short, long) = if a_chars.len() < b_chars.len() {
            (&a_chars, &b_chars)
        } else {
            (&b_chars, &a_chars)
        };

        let mut si = 0;
        let mut li = 0;
        let mut edits = 0;

        while si < short.len() && li < long.len() {
            if short[si] == long[li] {
                si += 1;
                li += 1;
            } else {
                edits += 1;
                if edits > 1 {
                    return false;
                }
                li += 1;
            }
        }

        true
    }
}

// =============================================================================
// Phase 1b: Jaccard n-gram containment
// =============================================================================

/// Asymmetric Jaccard containment using character bigrams.
pub fn jaccard_containment(query: &str, candidate: &str) -> f64 {
    let q = normalize(query);
    let c = normalize(candidate);
    let q_grams = bigrams_with_boundaries(&q);
    let c_grams = bigrams_with_boundaries(&c);

    if q_grams.is_empty() {
        return 0.0;
    }

    let intersection = q_grams.intersection(&c_grams).count();
    intersection as f64 / q_grams.len() as f64
}

fn bigrams_with_boundaries(text: &str) -> HashSet<[char; 2]> {
    let padded = format!("${text}$");
    let chars: Vec<char> = padded.chars().collect();
    let mut grams = HashSet::with_capacity(chars.len().saturating_sub(1));
    for window in chars.windows(2) {
        grams.insert([window[0], window[1]]);
    }
    grams
}

fn jaccard_from_bigrams(
    query_bigrams: &HashSet<[char; 2]>,
    candidate_bigrams: &HashSet<[char; 2]>,
) -> f64 {
    if query_bigrams.is_empty() {
        return 0.0;
    }
    let intersection = query_bigrams.intersection(candidate_bigrams).count();
    intersection as f64 / query_bigrams.len() as f64
}

// =============================================================================
// PreparedQuery — amortized query representation
// =============================================================================

/// Pre-compiled query for amortized fuzzy scoring.
///
/// Normalizes, tokenizes, and pre-computes character bigrams **once**.
/// Reuse across all candidates to eliminate O(N) redundant allocations.
#[derive(Clone, Debug)]
pub struct PreparedQuery {
    /// Normalized form: lowercase, separators→space, collapsed.
    pub normalized: String,
    /// Tokenized normalized form.
    pub tokens: Vec<String>,
    /// Character bigrams with boundary markers (for Jaccard containment).
    pub bigrams: HashSet<[char; 2]>,
}

impl PreparedQuery {
    /// Prepare a query for scoring. O(|query|) time and space.
    pub fn new(search: &str) -> Self {
        let normalized = normalize(search);
        let tokens = tokenize(&normalized);
        let bigrams = bigrams_with_boundaries(&normalized);
        Self {
            normalized,
            tokens,
            bigrams,
        }
    }
}

// =============================================================================
// NormalizedCorpus — pre-normalized DB values for amortized scoring
// =============================================================================

/// Pre-normalized corpus of DB values for amortized fuzzy scoring.
///
/// Normalizes and tokenizes each value once. Reuse across all queries
/// against the same column to eliminate O(N) redundant normalize() calls.
pub struct NormalizedCorpus {
    /// Original DB values (for result output).
    values: Vec<String>,
    /// Pre-computed normalized form of each value.
    normalized: Vec<String>,
    /// Pre-computed tokens of each normalized value.
    tokens: Vec<Vec<String>>,
    /// Pre-computed bigrams for Jaccard containment.
    bigrams: Vec<HashSet<[char; 2]>>,
}

impl NormalizedCorpus {
    /// Build a pre-normalized corpus from raw DB values. O(N) time.
    pub fn new(values: Vec<String>) -> Self {
        let normalized: Vec<String> = values.iter().map(|v| normalize(v)).collect();
        let tokens: Vec<Vec<String>> = normalized.iter().map(|n| tokenize(n)).collect();
        let bigrams: Vec<HashSet<[char; 2]>> = normalized
            .iter()
            .map(|n| bigrams_with_boundaries(n))
            .collect();
        Self {
            values,
            normalized,
            tokens,
            bigrams,
        }
    }

    /// Number of values in the corpus.
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Whether the corpus is empty.
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Access the raw DB values.
    pub fn values(&self) -> &[String] {
        &self.values
    }
}

// =============================================================================
// resolve_value — the main resolution pipeline
// =============================================================================

/// Score a single candidate against search term using Phase 1 signals only.
pub fn score_candidate_phase1(
    search_norm: &str,
    search_tokens: &[String],
    candidate: &str,
) -> Option<MatchScore> {
    let value_norm = normalize(candidate);
    let value_tokens = tokenize(&value_norm);
    score_candidate_phase1_prepared(
        search_norm,
        search_tokens,
        candidate,
        &value_norm,
        &value_tokens,
    )
}

fn score_candidate_phase1_prepared(
    search_norm: &str,
    search_tokens: &[String],
    candidate: &str,
    candidate_norm: &str,
    candidate_tokens: &[String],
) -> Option<MatchScore> {
    let mut signals: Vec<MatchSignal> = Vec::new();

    // Tier 1: Exact normalized match
    if candidate_norm == search_norm {
        signals.push(MatchSignal::Exact);
    } else {
        // Tier 1.5: Segment exact match
        if search_tokens.len() >= 2 {
            if let Some(idx) = segment_exact_match(search_norm, candidate) {
                signals.push(MatchSignal::SegmentExact { segment_index: idx });
            }
        }

        // Tier 2: Token containment
        if signals.is_empty() {
            if let Some(sig) = token_containment(search_tokens, candidate_tokens) {
                signals.push(sig);
            }
            // Tier 3: Token edit distance ≤ 1
            else if let Some(sig) = token_edit_match(search_tokens, candidate_tokens) {
                signals.push(sig);
            }
        }
    }

    if signals.is_empty() {
        None
    } else {
        Some(MatchScore::new(candidate.to_string(), signals))
    }
}

/// Amortized variant that accepts a pre-normalized corpus.
pub fn resolve_value_amortized(
    query: &PreparedQuery,
    corpus: &NormalizedCorpus,
    strategy: ResolveStrategy,
    vector_matches: Option<&[VectorMatch]>,
) -> Vec<MatchScore> {
    let threshold = strategy.threshold();
    let mut candidates: Vec<MatchScore> = Vec::new();
    let mut best_phase1_points: u32 = 0;
    let mut matched_indices: HashSet<usize> = HashSet::new();

    // Phase 1: Pure signals
    let early_exit = strategy == ResolveStrategy::Categorical;
    for i in 0..corpus.len() {
        if let Some(score) = score_candidate_phase1_prepared(
            &query.normalized,
            &query.tokens,
            &corpus.values[i],
            &corpus.normalized[i],
            &corpus.tokens[i],
        ) {
            if score.points > best_phase1_points {
                best_phase1_points = score.points;
            }
            if score.points >= threshold {
                if early_exit && score.points >= 100 {
                    return vec![score];
                }
                matched_indices.insert(i);
                candidates.push(score);
            }
        }
    }

    // Phase 1b: Jaccard n-gram fallback
    if best_phase1_points < 60 {
        for i in 0..corpus.len() {
            if matched_indices.contains(&i) || corpus.bigrams[i].is_empty() {
                continue;
            }
            let sim = jaccard_from_bigrams(&query.bigrams, &corpus.bigrams[i]);
            if sim >= 0.3 {
                let score = MatchScore::new(
                    corpus.values[i].clone(),
                    vec![MatchSignal::JaccardNgram { similarity: sim }],
                );
                if score.points >= threshold {
                    candidates.push(score);
                }
            }
        }
    }

    // Phase 2: Vector similarity (pre-computed by async callers)
    if let Some(vm) = vector_matches {
        let mut norm_to_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::with_capacity(candidates.len());
        for (i, c) in candidates.iter().enumerate() {
            norm_to_idx.insert(normalize(&c.value), i);
        }

        for hit in vm {
            let hit_norm = normalize(&hit.value);
            if let Some(&idx) = norm_to_idx.get(&hit_norm) {
                candidates[idx]
                    .signals
                    .push(MatchSignal::VectorSimilarity { cosine: hit.cosine });
                candidates[idx].points = candidates[idx].signals.iter().map(|s| s.points()).sum();
            } else {
                let score = MatchScore::new(
                    hit.value.clone(),
                    vec![MatchSignal::VectorSimilarity { cosine: hit.cosine }],
                );
                if score.points >= threshold {
                    let new_idx = candidates.len();
                    norm_to_idx.insert(hit_norm, new_idx);
                    candidates.push(score);
                }
            }
        }
    }

    candidates.sort_by(|a, b| b.points.cmp(&a.points));
    candidates
}

/// Resolve a search term against database column values using the point-based pipeline.
pub fn resolve_value(
    search: &str,
    db_values: &[String],
    strategy: ResolveStrategy,
    vector_matches: Option<&[VectorMatch]>,
) -> Vec<MatchScore> {
    let query = PreparedQuery::new(search);
    resolve_value_prepared(&query, db_values, strategy, vector_matches)
}

/// Amortized variant that accepts a pre-compiled [`PreparedQuery`].
pub fn resolve_value_prepared(
    query: &PreparedQuery,
    db_values: &[String],
    strategy: ResolveStrategy,
    vector_matches: Option<&[VectorMatch]>,
) -> Vec<MatchScore> {
    let corpus = NormalizedCorpus::new(db_values.to_vec());
    resolve_value_amortized(query, &corpus, strategy, vector_matches)
}

// =============================================================================
// RankedMatch — named return type for fuzzy ranking
// =============================================================================

/// A candidate value with its fuzzy match score.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RankedMatch {
    /// The matched database value.
    pub value: String,
    /// Match score (0.0 to 1.0).
    pub score: f64,
}

// =============================================================================
// Backward-compatible public API
// =============================================================================

/// Score how well `search` matches `candidate` (0.0 to 1.0).
pub fn fuzzy_score(search: &str, candidate: &str) -> f64 {
    let query = PreparedQuery::new(search);
    fuzzy_score_prepared(&query, candidate)
}

/// Amortized variant of [`fuzzy_score`].
pub fn fuzzy_score_prepared(query: &PreparedQuery, candidate: &str) -> f64 {
    let candidate_norm = normalize(candidate);

    if candidate_norm == query.normalized {
        return 1.0;
    }

    let candidate_tokens = tokenize(&candidate_norm);

    if let Some(sig) = token_containment(&query.tokens, &candidate_tokens) {
        return sig.points() as f64 / 100.0;
    }

    if token_edit_match(&query.tokens, &candidate_tokens).is_some() {
        return 0.60;
    }

    let c_grams = bigrams_with_boundaries(&candidate_norm);
    let jac = jaccard_from_bigrams(&query.bigrams, &c_grams);
    if jac > 0.0 {
        return jac * 0.4;
    }

    0.0
}

/// Find the best match for `input` among `values` above `threshold`.
pub fn find_best_match(input: &str, values: &[String], threshold: f64) -> Option<String> {
    let point_threshold = (threshold * 100.0) as u32;
    let results = resolve_value(input, values, ResolveStrategy::Categorical, None);
    results
        .into_iter()
        .find(|m| m.points >= point_threshold)
        .map(|m| m.value)
}

/// Find ALL matches for `input` among `values` at or above `threshold`.
pub fn find_all_matches(input: &str, values: &[String], threshold: f64) -> Vec<String> {
    let point_threshold = (threshold * 100.0) as u32;
    let results = resolve_value(input, values, ResolveStrategy::DisplayName, None);
    results
        .into_iter()
        .filter(|m| m.points >= point_threshold)
        .map(|m| m.value)
        .collect()
}

/// Weighted field for multi-field scoring.
pub struct ScoredField<'a> {
    pub name: &'a str,
    pub weight: f64,
    pub text: &'a str,
}

/// Best-scoring field from multi-field fuzzy matching.
pub struct MultiFieldScore<'a> {
    pub score: f64,
    pub field_name: &'a str,
}

/// Score a query against multiple weighted fields.
pub fn score_multi_field<'a>(query: &str, fields: &[ScoredField<'a>]) -> MultiFieldScore<'a> {
    fields
        .iter()
        .map(|f| MultiFieldScore {
            score: fuzzy_score(query, f.text) * f.weight,
            field_name: f.name,
        })
        .max_by(|a, b| {
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .unwrap_or(MultiFieldScore {
            score: 0.0,
            field_name: "",
        })
}

/// Result from multi-field point-based scoring.
#[derive(Clone, Debug)]
pub struct MultiFieldMatch {
    /// Which field produced the best match.
    pub field_name: String,
    /// Weight of the matching field.
    pub weight: f64,
    /// Raw points from the matching signal (0-100).
    pub points: u32,
    /// Normalized score: `(points * weight) / 100.0`.
    pub score: f64,
    /// Which signals contributed to this match.
    pub signals: Vec<MatchSignal>,
}

/// Score a query against multiple weighted fields using the point-based pipeline.
pub fn resolve_multi_field(
    search: &str,
    fields: &[ScoredField<'_>],
    strategy: ResolveStrategy,
) -> Option<MultiFieldMatch> {
    let query = PreparedQuery::new(search);
    resolve_multi_field_prepared(&query, fields, strategy)
}

/// Amortized variant of [`resolve_multi_field`].
pub fn resolve_multi_field_prepared(
    query: &PreparedQuery,
    fields: &[ScoredField<'_>],
    _strategy: ResolveStrategy,
) -> Option<MultiFieldMatch> {
    if fields.is_empty() {
        return None;
    }

    let field_norms: Vec<String> = fields.iter().map(|f| normalize(f.text)).collect();
    let field_tokens: Vec<Vec<String>> = field_norms.iter().map(|n| tokenize(n)).collect();

    let mut best: Option<MultiFieldMatch> = None;
    let mut best_phase1_points: u32 = 0;

    // Phase 1: Pure signals per field
    for (idx, field) in fields.iter().enumerate() {
        if let Some(ms) = score_candidate_phase1_prepared(
            &query.normalized,
            &query.tokens,
            field.text,
            &field_norms[idx],
            &field_tokens[idx],
        ) {
            let weighted_score = (ms.points as f64 * field.weight) / 100.0;
            if ms.points > best_phase1_points {
                best_phase1_points = ms.points;
            }
            let dominated = best.as_ref().is_some_and(|b| weighted_score <= b.score);
            if !dominated {
                best = Some(MultiFieldMatch {
                    field_name: field.name.to_string(),
                    weight: field.weight,
                    points: ms.points,
                    score: weighted_score,
                    signals: ms.signals,
                });
            }
        }
    }

    // Phase 1b: Jaccard fallback per field (only if Phase 1 max < 60)
    if best_phase1_points < 60 {
        for (idx, field) in fields.iter().enumerate() {
            let c_grams = bigrams_with_boundaries(&field_norms[idx]);
            let sim = jaccard_from_bigrams(&query.bigrams, &c_grams);
            if sim >= 0.3 {
                let points = (sim * 40.0) as u32;
                let weighted_score = (points as f64 * field.weight) / 100.0;
                let dominated = best.as_ref().is_some_and(|b| weighted_score <= b.score);
                if !dominated {
                    let signals = vec![MatchSignal::JaccardNgram { similarity: sim }];
                    best = Some(MultiFieldMatch {
                        field_name: field.name.to_string(),
                        weight: field.weight,
                        points,
                        score: weighted_score,
                        signals,
                    });
                }
            }
        }
    }

    best
}

/// Rank all matches for `input` among `values`, returning top `limit` scored pairs.
pub fn rank_matches(input: &str, values: &[String], limit: usize) -> Vec<RankedMatch> {
    let query = PreparedQuery::new(input);
    let mut scored: Vec<RankedMatch> = values
        .iter()
        .map(|v| RankedMatch {
            value: v.clone(),
            score: fuzzy_score_prepared(&query, v),
        })
        .filter(|m| m.score > 0.1)
        .collect();

    scored.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    scored.truncate(limit);
    scored
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Normalization ─────────────────────────────────────────────────────

    #[test]
    fn normalize_folds_case_and_separators() {
        assert_eq!(normalize("Brand_2"), "brand 2");
        assert_eq!(normalize("brand-2"), "brand 2");
        assert_eq!(normalize("BRAND 2"), "brand 2");
        assert_eq!(normalize("  multiple   spaces  "), "multiple spaces");
        assert_eq!(normalize("ice_cream-handheld"), "ice cream handheld");
    }

    // ── Exact match ──────────────────────────────────────────────────────

    #[test]
    fn exact_match_scores_100() {
        assert_eq!(fuzzy_score("jalapeno", "jalapeno"), 1.0);
        assert_eq!(fuzzy_score("Jalapeno", "jalapeno"), 1.0);
    }

    #[test]
    fn underscore_space_hyphen_all_equivalent() {
        assert_eq!(fuzzy_score("premium kits", "premium_kits"), 1.0);
        assert_eq!(fuzzy_score("premium_kits", "premium kits"), 1.0);
        assert_eq!(fuzzy_score("brand-2", "brand_2"), 1.0);
        assert_eq!(fuzzy_score("Brand 2", "brand-2"), 1.0);
        assert_eq!(fuzzy_score("Standard Kits", "standard_kits"), 1.0);
    }

    // ── Token containment ────────────────────────────────────────────────

    #[test]
    fn token_containment_matches_whole_tokens() {
        let score = fuzzy_score("taco", "mexican-taco-mild-375g");
        assert!(
            score >= 0.80 && score <= 0.98,
            "Expected 0.80-0.98, got {}",
            score
        );
    }

    #[test]
    fn token_containment_multi_token() {
        let score = fuzzy_score("premium kits", "super_premium_kits");
        assert!(score >= 0.80, "Expected ≥0.80, got {}", score);
    }

    // ── Numeric token exactness ─────────────────────────────────────────

    #[test]
    fn brand_2_does_not_match_brand_20() {
        let score = fuzzy_score("Brand 2", "Brand 20");
        assert!(
            score < 0.5,
            "Brand 2 must NOT match Brand 20, got {}",
            score
        );
    }

    #[test]
    fn brand_2_exactly_matches_brand_2() {
        assert_eq!(fuzzy_score("Brand 2", "Brand 2"), 1.0);
    }

    // ── Edit distance ────────────────────────────────────────────────────

    #[test]
    fn edit_distance_handles_typo() {
        let score = fuzzy_score("bradn 2", "brand 2");
        assert!(
            (score - 0.60).abs() < 0.01,
            "Expected ~0.60 for typo, got {}",
            score
        );
    }

    #[test]
    fn edit_distance_numeric_still_exact() {
        let score = fuzzy_score("Brand 2", "Brand 3");
        assert!(score < 0.5, "Numeric tokens must be exact, got {}", score);
    }

    // ── Jaccard n-gram ───────────────────────────────────────────────────

    #[test]
    fn jaccard_containment_high_for_substrings() {
        let jac = jaccard_containment("taco", "mexican-taco-mild");
        assert!(
            jac > 0.4,
            "Expected high jaccard for taco in taco-mild, got {}",
            jac
        );
    }

    #[test]
    fn jaccard_containment_low_for_unrelated() {
        let jac = jaccard_containment("xyz", "abc");
        assert!(jac < 0.2, "Expected low jaccard for unrelated, got {}", jac);
    }

    // ── find_best_match ──────────────────────────────────────────────────

    #[test]
    fn find_best_match_prefers_exact() {
        let values = vec![
            "Brand 1".to_string(),
            "Brand 2".to_string(),
            "Brand 20".to_string(),
        ];
        let result = find_best_match("Brand 2", &values, 0.3);
        assert_eq!(result, Some("Brand 2".to_string()));
    }

    #[test]
    fn find_best_match_rejects_numeric_prefix_collision() {
        let values = vec!["Brand 20".to_string(), "Brand 29".to_string()];
        let result = find_best_match("Brand 2", &values, 0.6);
        assert!(
            result.is_none(),
            "Brand 2 must not match Brand 20, got {:?}",
            result
        );
    }

    // ── find_all_matches ─────────────────────────────────────────────────

    #[test]
    fn find_all_matches_returns_all_token_matches() {
        let values = vec![
            "mexican-kits-taco-mild-375g".to_string(),
            "mexican-shells-taco-160g".to_string(),
            "mexican-spice-taco-30g".to_string(),
            "ice_cream-handheld-multi".to_string(),
            "baking-treats-chocolate".to_string(),
        ];
        let result = find_all_matches("taco", &values, 0.3);
        assert_eq!(result.len(), 3, "Expected 3 taco matches, got {:?}", result);
    }

    // ── resolve_value (point-based) ──────────────────────────────────────

    #[test]
    fn resolve_value_exact_match_100_points() {
        let values = vec!["Brand 2".to_string(), "Brand 20".to_string()];
        let results = resolve_value("Brand 2", &values, ResolveStrategy::Categorical, None);
        assert!(!results.is_empty());
        assert_eq!(results[0].value, "Brand 2");
        assert_eq!(results[0].points, 100);
    }

    #[test]
    fn resolve_value_token_containment() {
        let values = vec![
            "mexican-taco-mild-375g".to_string(),
            "ice-cream-vanilla".to_string(),
        ];
        let results = resolve_value("taco", &values, ResolveStrategy::DisplayName, None);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].value, "mexican-taco-mild-375g");
        assert!(results[0].points >= 80);
    }

    #[test]
    fn resolve_value_empty_search_returns_empty() {
        let values = vec!["something".to_string()];
        let results = resolve_value("", &values, ResolveStrategy::Categorical, None);
        assert!(results.is_empty());
    }

    // ── rank_matches ─────────────────────────────────────────────────────

    #[test]
    fn rank_matches_sorted_by_score() {
        let values = vec![
            "jalapeno".to_string(),
            "jalapenos_chillies".to_string(),
            "bell_peppers".to_string(),
        ];
        let ranked = rank_matches("jalapeno", &values, 5);
        assert!(!ranked.is_empty());
        assert_eq!(ranked[0].value, "jalapeno");
        assert_eq!(ranked[0].score, 1.0);
    }

    #[test]
    fn rank_matches_respects_limit() {
        let values = vec![
            "a_jalapeno".to_string(),
            "b_jalapeno".to_string(),
            "c_jalapeno".to_string(),
        ];
        let ranked = rank_matches("jalapeno", &values, 2);
        assert_eq!(ranked.len(), 2);
    }

    // ── score_multi_field ────────────────────────────────────────────────

    #[test]
    fn score_multi_field_picks_highest_weighted() {
        let fields = vec![
            ScoredField {
                name: "name",
                weight: 1.0,
                text: "dim_products",
            },
            ScoredField {
                name: "description",
                weight: 0.8,
                text: "Product dimension table",
            },
        ];
        let result = score_multi_field("products", &fields);
        assert_eq!(result.field_name, "name");
        assert!(result.score > 0.3);
    }

    #[test]
    fn score_multi_field_empty_returns_zero() {
        let result = score_multi_field("anything", &[]);
        assert_eq!(result.score, 0.0);
        assert_eq!(result.field_name, "");
    }

    // ── edit_distance_at_most_one ────────────────────────────────────────

    #[test]
    fn edit_distance_exact() {
        assert!(edit_distance_at_most_one("brand", "brand"));
    }

    #[test]
    fn edit_distance_substitution() {
        assert!(edit_distance_at_most_one("cat", "bat"));
    }

    #[test]
    fn edit_distance_transposition() {
        assert!(edit_distance_at_most_one("brand", "bradn"));
    }

    #[test]
    fn edit_distance_insertion() {
        assert!(edit_distance_at_most_one("taco", "tacos"));
        assert!(edit_distance_at_most_one("jalapeno", "jalapenos"));
    }

    #[test]
    fn edit_distance_too_far() {
        assert!(edit_distance_at_most_one("brand", "bland"));
        assert!(!edit_distance_at_most_one("abc", "xyz"));
        assert!(!edit_distance_at_most_one("hello", "world"));
    }

    // ── MatchSignal::label ──────────────────────────────────────────────

    #[test]
    fn match_signal_label_exact() {
        assert_eq!(MatchSignal::Exact.label(), "exact");
    }

    #[test]
    fn match_signal_label_token() {
        let sig = MatchSignal::TokenContainment {
            matched_tokens: 2,
            total_candidate_tokens: 4,
        };
        assert_eq!(sig.label(), "token(2/4)");
    }

    // ── segment_exact_match ──────────────────────────────────────────────

    #[test]
    fn segment_exact_match_finds_brand_2() {
        let result = segment_exact_match(
            "brand 2",
            "baking-cakes - brand-2 - sub-brand-5 - sweet - 340g",
        );
        assert_eq!(result, Some(1));
    }

    #[test]
    fn segment_exact_match_rejects_sub_brand_2() {
        let result = segment_exact_match(
            "brand 2",
            "baking-cakes - brand-5 - sub-brand-2 - sweet - 340g",
        );
        assert_eq!(result, None);
    }

    // ── Vector similarity ────────────────────────────────────────────────

    #[test]
    fn resolve_value_with_vector_match_adds_signal() {
        let values = vec![
            "mexican-taco-mild-375g".to_string(),
            "ice-cream-vanilla".to_string(),
        ];
        let vm = vec![VectorMatch {
            value: "mexican-taco-mild-375g".to_string(),
            cosine: 0.85,
        }];
        let results = resolve_value("taco", &values, ResolveStrategy::DisplayName, Some(&vm));
        assert!(!results.is_empty());
        let best = &results[0];
        assert_eq!(best.value, "mexican-taco-mild-375g");
        let has_vector = best
            .signals
            .iter()
            .any(|s| matches!(s, MatchSignal::VectorSimilarity { .. }));
        assert!(has_vector);
    }

    #[test]
    fn resolve_value_with_vector_match_new_candidate() {
        let values = vec!["carbonated_drinks".to_string(), "still_water".to_string()];
        let vm = vec![VectorMatch {
            value: "carbonated_drinks".to_string(),
            cosine: 0.82,
        }];
        let results = resolve_value("soda", &values, ResolveStrategy::DisplayName, Some(&vm));
        assert!(!results.is_empty());
        assert_eq!(results[0].value, "carbonated_drinks");
    }

    // ── PreparedQuery ──────────────────────────────────────────────────

    #[test]
    fn prepared_query_normalizes_and_tokenizes() {
        let pq = PreparedQuery::new("Ice_Cream-TACO");
        assert_eq!(pq.normalized, "ice cream taco");
        assert_eq!(pq.tokens, vec!["ice", "cream", "taco"]);
        assert!(!pq.bigrams.is_empty());
    }

    #[test]
    fn prepared_query_empty_string() {
        let pq = PreparedQuery::new("");
        assert_eq!(pq.normalized, "");
        assert!(pq.tokens.is_empty());
    }

    // ── NormalizedCorpus equivalence ────────────────────────────────────

    #[test]
    fn normalized_corpus_matches_raw() {
        let values = vec![
            "Brand 2".to_string(),
            "Brand 20".to_string(),
            "mexican-taco-mild-375g".to_string(),
            "ice_cream-handheld".to_string(),
            "premium_kits".to_string(),
        ];
        let corpus = NormalizedCorpus::new(values.clone());
        let queries = ["Brand 2", "taco", "premium kits", "xyz"];

        for search in &queries {
            let pq = PreparedQuery::new(search);
            let raw = resolve_value_prepared(&pq, &values, ResolveStrategy::DisplayName, None);
            let amortized =
                resolve_value_amortized(&pq, &corpus, ResolveStrategy::DisplayName, None);

            assert_eq!(raw.len(), amortized.len());
            for (r, a) in raw.iter().zip(amortized.iter()) {
                assert_eq!(r.value, a.value);
                assert_eq!(r.points, a.points);
            }
        }
    }

    // ── resolve_multi_field ─────────────────────────────────────────────

    #[test]
    fn resolve_multi_field_picks_best_weighted_field() {
        let fields = vec![
            ScoredField {
                name: "name",
                weight: 1.0,
                text: "dim_products",
            },
            ScoredField {
                name: "description",
                weight: 0.8,
                text: "products table",
            },
        ];
        let m = resolve_multi_field("products", &fields, ResolveStrategy::DisplayName);
        assert!(m.is_some());
        let m = m.unwrap();
        assert_eq!(m.field_name, "name");
        assert!(m.score > 0.8);
    }

    #[test]
    fn resolve_multi_field_empty_fields() {
        let m = resolve_multi_field("anything", &[], ResolveStrategy::DisplayName);
        assert!(m.is_none());
    }

    // ── collect_matched_values ────────────────────────────────────────

    fn make_match(value: &str, points: u32) -> MatchScore {
        MatchScore {
            points,
            signals: vec![MatchSignal::Exact],
            value: value.to_string(),
        }
    }

    #[test]
    fn collect_matched_values_empty_returns_empty() {
        // L1: Empty preservation
        let result = collect_matched_values(vec![], ResolveStrategy::Categorical);
        assert!(result.is_empty());
        let result = collect_matched_values(vec![], ResolveStrategy::DisplayName);
        assert!(result.is_empty());
    }

    #[test]
    fn collect_matched_values_categorical_returns_single_best() {
        // L2: Categorical bound
        let matches = vec![
            make_match("Brand 1", 100),
            make_match("Brand 2", 80),
            make_match("Brand 3", 60),
        ];
        let result = collect_matched_values(matches, ResolveStrategy::Categorical);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], "Brand 1");
    }

    #[test]
    fn collect_matched_values_display_name_returns_all() {
        // L3: DisplayName totality
        let matches = vec![
            make_match("taco-mild", 90),
            make_match("taco-hot", 85),
            make_match("taco-supreme", 80),
        ];
        let result = collect_matched_values(matches, ResolveStrategy::DisplayName);
        assert_eq!(result.len(), 3);
        assert_eq!(result[0], "taco-mild");
        assert_eq!(result[1], "taco-hot");
        assert_eq!(result[2], "taco-supreme");
    }

    #[test]
    fn collect_matched_values_best_always_included() {
        // L4: Best inclusion
        let matches = vec![make_match("best-value", 100)];
        let cat = collect_matched_values(matches.clone(), ResolveStrategy::Categorical);
        assert!(cat.contains(&"best-value".to_string()));
        let dn = collect_matched_values(matches, ResolveStrategy::DisplayName);
        assert!(dn.contains(&"best-value".to_string()));
    }
}

/// Reference normalize for hegel equivalence verification.
#[cfg(test)]
fn normalize_reference(s: &str) -> String {
    s.to_lowercase()
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod hegel_tests {
    use super::*;
    use hegel::generators;

    #[hegel::test]
    fn normalize_equivalence(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text().min_size(1).max_size(10));
        assert_eq!(normalize(&s), normalize_reference(&s));
    }

    #[hegel::test]
    fn normalize_idempotent(tc: hegel::TestCase) {
        let s: String = tc.draw(generators::text().min_size(1).max_size(10));
        let once = normalize(&s);
        let twice = normalize(&once);
        assert_eq!(once, twice);
    }

    #[hegel::test]
    fn points_bounded_jaccard(tc: hegel::TestCase) {
        let similarity = tc.draw(generators::floats::<f64>());
        let signal = MatchSignal::JaccardNgram { similarity };
        let p = signal.points();
        assert!(p <= 100);
    }

    #[hegel::test]
    fn points_bounded_vector(tc: hegel::TestCase) {
        let cosine = tc.draw(generators::floats::<f64>());
        let signal = MatchSignal::VectorSimilarity { cosine };
        let p = signal.points();
        assert!(p <= 100);
    }

    #[hegel::test]
    fn points_bounded_token_containment(tc: hegel::TestCase) {
        let matched = tc.draw(generators::integers::<u32>().min_value(0).max_value(99));
        let total = tc.draw(generators::integers::<u32>().min_value(0).max_value(99));
        let signal = MatchSignal::TokenContainment {
            matched_tokens: matched,
            total_candidate_tokens: total,
        };
        let p = signal.points();
        assert!(p <= 100);
    }

    // L4: Best inclusion — first element always in result for both strategies
    #[hegel::test]
    fn collect_matched_values_best_inclusion(tc: hegel::TestCase) {
        let n = tc.draw(generators::integers::<usize>().min_value(1).max_value(9));
        let matches: Vec<MatchScore> = (0..n)
            .map(|i| MatchScore {
                points: 100 - i as u32,
                signals: vec![MatchSignal::Exact],
                value: format!("value_{}", i),
            })
            .collect();
        let best = matches[0].value.clone();

        let cat = collect_matched_values(matches.clone(), ResolveStrategy::Categorical);
        assert!(cat.contains(&best));

        let dn = collect_matched_values(matches, ResolveStrategy::DisplayName);
        assert!(dn.contains(&best));
    }
}
