/**
 * Scores `candidate` against `query` as a case-insensitive subsequence, or returns
 * `null` if the query's characters do not appear in order. Higher is better:
 * consecutive characters, characters at the start of a path segment or word, and
 * matches in the file name score more; long candidates score less.
 */
export function fuzzyScore(query: string, candidate: string): number | null {
  const q = query.toLowerCase().replace(/\s+/g, "");
  if (q === "") return 0;
  const c = candidate.toLowerCase();
  const nameStart = candidate.lastIndexOf("/") + 1;
  let score = 0;
  let from = 0;
  let previous = -2;
  for (const char of q) {
    const index = c.indexOf(char, from);
    if (index < 0) return null;
    score += 1;
    if (index === previous + 1) score += 5;
    if (index === 0 || "/._- ".includes(c[index - 1]!)) score += 3;
    if (index >= nameStart) score += 2;
    previous = index;
    from = index + 1;
  }
  return score - candidate.length * 0.01;
}

/** The best `limit` matches, best first. */
export function fuzzyFilter<T>(query: string, items: readonly T[], text: (item: T) => string, limit = 50): T[] {
  const scored: Array<{ item: T; score: number }> = [];
  for (const item of items) {
    const score = fuzzyScore(query, text(item));
    if (score !== null) scored.push({ item, score });
  }
  return scored
    .sort((a, b) => b.score - a.score)
    .slice(0, limit)
    .map((s) => s.item);
}
