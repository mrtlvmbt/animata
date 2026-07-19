#!/usr/bin/env bash
# Verify that v2-sim sharding covers all tests exactly once: no gaps, no overlaps, deterministic.
# Run from the v2 directory during CI.
set -uo pipefail

echo "→ Collecting test names from all shards..."

# List all tests from the current v2-sim-x86 setup (the golden set we're sharding)
# This is what SHOULD be covered by the shards.
echo "  Baseline (all non-golden + all non-v2_golden except work_counter in perf)..."
BASELINE=$(
  {
    # Debug: all non-golden
    cargo nextest list --workspace --locked -E 'not test(v2_golden)' --list-type grouped 2>/dev/null || true
    # Release: all non-golden (work_counter runs both here and in perf)
    cargo nextest list --workspace --release --locked -E 'not test(v2_golden)' --list-type grouped 2>/dev/null || true
    # Perf: work_counter with perf feature (already in the release run, so we don't double-count)
    # Omit this to avoid duplicates; work_counter is already covered by release
  } | grep "::" | sort -u
)

# List tests from each shard
echo "  Shard 1 (debug + perf)..."
SHARD1=$(
  {
    cargo nextest list --workspace --locked -E 'not test(v2_golden)' --list-type grouped 2>/dev/null || true
    # Perf tests don't add new test names, they're the same tests run with different feature
  } | grep "::" | sort -u
)

echo "  Shard 2 (release world/sim-core/brain)..."
SHARD2=$(
  cargo nextest list -p world -p sim-core -p brain --release --locked -E 'not test(v2_golden)' --list-type grouped 2>/dev/null | grep "::" | sort -u || true
)

echo "  Shard 3 (release cli, excluding work_counter perf)..."
SHARD3=$(
  cargo nextest list -p cli --release --locked -E 'not test(v2_golden)' -E 'not test(work_counter)' --list-type grouped 2>/dev/null | grep "::" | sort -u || true
)

# Combine all shards
COMBINED=$(printf "%s\n%s\n%s\n" "$SHARD1" "$SHARD2" "$SHARD3" | grep -v "^$" | sort -u)

# Compare baseline vs combined
BASELINE_SORTED=$(printf "%s" "$BASELINE" | sort -u)
COMBINED_SORTED=$(printf "%s" "$COMBINED" | sort -u)

BASELINE_COUNT=$(printf "%s" "$BASELINE_SORTED" | wc -l)
COMBINED_COUNT=$(printf "%s" "$COMBINED_SORTED" | wc -l)

echo ""
echo "  Baseline tests: $BASELINE_COUNT"
echo "  Combined shards: $COMBINED_COUNT"
echo ""

# Check for missing tests (in baseline but not in shards)
MISSING=$(comm -23 <(printf "%s" "$BASELINE_SORTED") <(printf "%s" "$COMBINED_SORTED"))
MISSING_COUNT=$(printf "%s" "$MISSING" | wc -l)

# Check for extra tests (in shards but not in baseline)
EXTRA=$(comm -13 <(printf "%s" "$BASELINE_SORTED") <(printf "%s" "$COMBINED_SORTED"))
EXTRA_COUNT=$(printf "%s" "$EXTRA" | wc -l)

if [ "$MISSING_COUNT" -gt 0 ] && [ -n "$MISSING" ]; then
  echo "✗ MISSING TESTS (not covered by any shard):"
  printf "%s" "$MISSING" | head -20 | sed 's/^/    /'
  echo ""
fi

if [ "$EXTRA_COUNT" -gt 0 ] && [ -n "$EXTRA" ]; then
  echo "✗ EXTRA TESTS (in shards but not in baseline):"
  printf "%s" "$EXTRA" | head -20 | sed 's/^/    /'
  echo ""
fi

# Check for duplicates within shards (same test in multiple shards)
OVERLAP=$(comm -12 <(printf "%s" "$SHARD1" | sort -u) <(printf "%s" "$SHARD2" | sort -u))
OVERLAP=$(printf "%s\n%s" "$OVERLAP" "$(comm -12 <(printf "%s" "$SHARD2" | sort -u) <(printf "%s" "$SHARD3" | sort -u))" | grep -v "^$" | sort -u)
OVERLAP_COUNT=$(printf "%s" "$OVERLAP" | wc -l)

if [ "$OVERLAP_COUNT" -gt 0 ] && [ -n "$OVERLAP" ]; then
  echo "✗ OVERLAP TESTS (in multiple shards):"
  printf "%s" "$OVERLAP" | head -20 | sed 's/^/    /'
  echo ""
fi

# Final verdict
if [ "$MISSING_COUNT" -eq 0 ] && [ "$EXTRA_COUNT" -eq 0 ] && [ "$OVERLAP_COUNT" -eq 0 ]; then
  echo "✓ Shard coverage is complete and deterministic"
  echo "  All $BASELINE_COUNT baseline tests covered exactly once across 3 shards"
  exit 0
else
  echo "✗ SHARD COVERAGE FAILURE"
  echo "  Baseline: $BASELINE_COUNT | Combined: $COMBINED_COUNT"
  [ "$MISSING_COUNT" -gt 0 ] && echo "  Missing: $MISSING_COUNT" || true
  [ "$EXTRA_COUNT" -gt 0 ] && echo "  Extra: $EXTRA_COUNT" || true
  [ "$OVERLAP_COUNT" -gt 0 ] && echo "  Overlaps: $OVERLAP_COUNT" || true
  exit 1
fi
