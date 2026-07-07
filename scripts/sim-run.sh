#!/usr/bin/env bash
# Диспатчит сценарий sim-run.yml и тянет результат — sim-run-аналог ci-report.sh.
#   scripts/sim-run.sh <scenario> [k=v ...]
#   e.g.  scripts/sim-run.sh perf seed=1 ticks=4000 bench_pop=200000
#         scripts/sim-run.sh v2-perf seed=1 ticks=400 bench_pop=5000  # v2 per-stage breakdown
#         scripts/sim-run.sh sweep param=photo_rate values=0.5,1,2 seeds=1,2,3 force=true
#         scripts/sim-run.sh gridsweep param=photo_rate values=0.5,1,2,4 seeds=1,2  # one runner per value
#
# perf vs v2-perf: both profile phase-timing, but `perf` runs the V1 headless binary (root workspace)
# while `v2-perf` builds v2-sim from the v2/ standalone workspace with --features perf (required for
# the per-stage table) and runs it via --bench-pop N --profile. v2-perf fails loud if the breakdown
# is missing (guard against a forgotten --features perf).
#
# sweep vs gridsweep: same single-param value grid, but `sweep` runs the cells SERIALLY in one job while
# `gridsweep` fans each value onto its OWN runner (a job matrix) so a wide grid finishes in ~1× per-cell
# wallclock. Both download into the same per-nonce dir (gridsweep lands one artifact per value).
# Exit: 0 = прогон success, 1 = прогон провалился, 2 = usage/инфра/таймаут.
#
# ВАЖНО: требует, чтобы sim-run.yml со сценарием/инпутами УЖЕ был на main — workflow_dispatch читает
# декларацию inputs из main; `-f` для не-объявленного там ключа API молча проглотит.
set -uo pipefail

WORKFLOW="sim-run.yml"
OUT_BASE=".sim-run"               # OUT_DIR = OUT_BASE/<nonce> (per-nonce → параллельные sim-run.sh & не затирают друг друга).
CAP="${SIM_RUN_TIMEOUT:-21600}"   # 6 ч — sim-прогоны длинные; НЕ наследуем 30-мин cap из ci-report.sh.
VALID_KEYS="scenario seed ticks bench_pop param values seeds force run_nonce"

die() { echo "✗ $*" >&2; exit 2; }

[ $# -ge 1 ] || die "usage: sim-run.sh <scenario> [k=v ...]  (scenario: evo-stats|perf|v2-perf|multiseed|sweep|gridsweep|dprime-2c|dprime-3b|driver-emergence|hypoxia-verdict|settling-verdict|dol-verdict|composition-verdict|dr0-diag|dr0-gradient)"
SCENARIO="$1"; shift

command -v gh >/dev/null 2>&1 || die "gh CLI не найден"
# workflow_dispatch через gh требует scope 'workflow' у токена — детектим заранее, не ловим 403 в цикле.
if ! gh auth status 2>&1 | grep -qi "workflow"; then
  die "у gh-токена нет scope 'workflow' (нужен для диспатча). Выполни: gh auth refresh -s workflow"
fi

# Собираем -f из k=v с whitelist (опечатанный ключ API иначе молча игнорит → дефолтный прогон).
FARGS=(-f "scenario=$SCENARIO")
for kv in "$@"; do
  k="${kv%%=*}"
  case " $VALID_KEYS " in
    *" $k "*) FARGS+=(-f "$kv") ;;
    *) die "неизвестный input-ключ '$k' (valid: $VALID_KEYS)" ;;
  esac
done

# Нонс → в run_nonce → в run-name, чтобы найти ИМЕННО наш прогон (часо-независимо, без гонки).
NONCE="$(date +%s)-$$-${RANDOM}"
FARGS+=(-f "run_nonce=$NONCE")
OUT_DIR="$OUT_BASE/$NONCE"   # уникальна по нонсу → несколько фоновых sim-run.sh не дерутся за артефакты

echo "→ Диспатч $WORKFLOW scenario=$SCENARIO nonce=$NONCE ..."
gh workflow run "$WORKFLOW" --ref main "${FARGS[@]}" || die "gh workflow run упал"

# Находим НАШ run по нонсу в displayTitle (gh workflow run не возвращает id; dispatch-run не привязан
# к HEAD-SHA, поэтому матч по коммиту, как в ci-report.sh, тут невозможен).
echo "→ Ищу run с nonce [$NONCE] ..."
RUN_ID=""
for _ in $(seq 1 30); do
  RUN_ID="$(gh run list --workflow "$WORKFLOW" --event workflow_dispatch -L 20 \
    --json databaseId,displayTitle \
    --jq "map(select(.displayTitle | contains(\"[$NONCE]\"))) | .[0].databaseId" 2>/dev/null)"
  [ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ] && break
  sleep 4
done
[ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ] || die "run с nonce $NONCE не найден (диспатч прошёл? workflow на main?)"

# Ждём до РЕАЛЬНОГО completed (gh run watch выходит рано на queued — крутим, как в ci-report.sh).
echo "→ Жду завершения run #$RUN_ID (cap ${CAP}s) ..."
DEADLINE=$(( $(date +%s) + CAP ))
while :; do
  STATUS="$(gh run view "$RUN_ID" --json status --jq .status 2>/dev/null)"
  [ "$STATUS" = "completed" ] && break
  if [ "$(date +%s)" -ge "$DEADLINE" ]; then
    echo "✗ Таймаут (cap ${CAP}s); run #$RUN_ID ещё ${STATUS:-unknown} — он, вероятно, ЖИВ, глянь в Actions" >&2
    exit 2
  fi
  gh run watch "$RUN_ID" --exit-status >/dev/null 2>&1 || true
  sleep 5
done

CONCLUSION="$(gh run view "$RUN_ID" --json conclusion --jq .conclusion 2>/dev/null)"
URL="$(gh run view "$RUN_ID" --json url --jq .url 2>/dev/null)"
rm -rf "$OUT_DIR" && mkdir -p "$OUT_DIR"
gh run download "$RUN_ID" -D "$OUT_DIR" 2>/dev/null || true

echo "run:        #$RUN_ID"
echo "conclusion: ${CONCLUSION:-unknown}"
echo "url:        $URL"
echo "─── output (summary) ───"
find "$OUT_DIR" -name summary.txt -exec cat {} \; 2>/dev/null | head -60
echo "(артефакты в $OUT_DIR/)"

[ "$CONCLUSION" = "success" ] && exit 0
exit 1
