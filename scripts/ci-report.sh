#!/usr/bin/env bash
# Запускать ПОСЛЕ git push. Находит CI-run для текущего HEAD, ждёт его и
# собирает машиночитаемый отчёт для агента.
# Exit code: 0 = success, 1 = тесты упали, 2 = ошибка инфраструктуры/таймаут.
#
# ВАЖНО для вызывающего: exit 0 — гарантия зелёного ТОЛЬКО если код не потерян пайпом.
# `ci-report.sh | tail` возвращает код `tail` (0 ВСЕГДА), маскируя падение. Запускай БЕЗ пайпа и
# читай `$?`, ЛИБО грепай последнюю строку `VERDICT: GREEN|RED` (она печатается в самый конец и
# переживает `| tail`), ЛИБО читай `.ci-report/summary.txt`.
set -uo pipefail

# Префлайт: на ЛЮБОМ хосте нужен залогиненный gh (scope repo), иначе gh-вызовы ниже тихо фейлят и
# скрипт ложно сообщит «не нашёл run» вместо реальной причины. Падаем рано с понятной инструкцией.
command -v gh >/dev/null 2>&1 || { echo "✗ gh CLI не найден — установи gh и выполни: gh auth login" >&2; exit 2; }
gh auth status >/dev/null 2>&1 || { echo "✗ gh не авторизован на этом хосте — выполни: gh auth login (scope repo)" >&2; exit 2; }

WORKFLOW="tests.yml"
OUT_DIR=".ci-report"
SHA="$(git rev-parse HEAD)"
rm -rf "$OUT_DIR" && mkdir -p "$OUT_DIR"

echo "→ Ищу CI-run для commit ${SHA:0:8} ..."
RUN_ID=""
for _ in $(seq 1 30); do          # до ~2 минут ожидания регистрации run
  RUN_ID="$(gh run list --workflow "$WORKFLOW" --limit 30 \
    --json databaseId,headSha \
    --jq "map(select(.headSha == \"$SHA\")) | .[0].databaseId" 2>/dev/null)"
  [ -n "$RUN_ID" ] && [ "$RUN_ID" != "null" ] && break
  sleep 4
done

if [ -z "$RUN_ID" ] || [ "$RUN_ID" = "null" ]; then
  echo "✗ Не нашёл run для $SHA (push прошёл? workflow на месте?)" | tee "$OUT_DIR/summary.txt"
  exit 2
fi

echo "→ Жду завершения run #$RUN_ID ..."
# `gh run watch` ВОЗВРАЩАЕТСЯ СРАЗУ, если run ещё `queued` и джобы не созданы (ровно состояние
# сразу после push). Поэтому не доверяем одному watch: крутим, пока status реально не станет
# `completed` (или не выйдет таймаут). Как только джобы появятся, watch начнёт блокировать сам.
DEADLINE=$(( $(date +%s) + 1800 ))   # 30 мин потолок ожидания
while :; do
  STATUS="$(gh run view "$RUN_ID" --json status --jq .status 2>/dev/null)"
  [ "$STATUS" = "completed" ] && break
  if [ "$(date +%s)" -ge "$DEADLINE" ]; then
    echo "✗ Таймаут ожидания run #$RUN_ID (status=${STATUS:-unknown})" | tee "$OUT_DIR/summary.txt"
    exit 2
  fi
  gh run watch "$RUN_ID" --exit-status >/dev/null 2>&1 || true
  sleep 5
done

# Метаданные прогона — извлекаем СТРОГО, чтобы exit 0 был настоящей гарантией зелёного для ИМЕННО
# этого commit: агрегатный conclusion, headSha самого run (не чужой ли прогон) и conclusion КАЖДОГО
# джоба отдельно (агрегат теоретически может соврать / прогон мог быть частичным).
gh run view "$RUN_ID" \
  --json status,conclusion,displayTitle,headSha,url,jobs \
  > "$OUT_DIR/run.json" 2>/dev/null
CONCLUSION="$(gh run view "$RUN_ID" --json conclusion --jq '.conclusion // ""' 2>/dev/null)"
RUN_SHA="$(gh run view "$RUN_ID" --json headSha --jq '.headSha // ""' 2>/dev/null)"
URL="$(gh run view "$RUN_ID" --json url --jq '.url // ""' 2>/dev/null)"
# Каждый джоб обязан быть success или skipped (path gating); ПУСТОЙ список джобов ⇒ НЕ зелёный (jq `all` на [] даёт true — гасим).
ALL_JOBS_OK="$(gh run view "$RUN_ID" --json jobs \
  --jq 'if (.jobs | length) > 0 then ([.jobs[].conclusion] | all(. == "success" or . == "skipped")) else false end' 2>/dev/null)"

# Структура: какие тесты упали (JUnit XML). Два джоба (x86 + arm64) кладут по своему артефакту
# (test-report-x86 / test-report-arm64) — тянем ВСЕ в .ci-report/artifacts/<name>/junit.xml.
gh run download "$RUN_ID" -D "$OUT_DIR/artifacts" 2>/dev/null || true

{
  echo "commit:     $SHA"
  echo "run:        #$RUN_ID"
  echo "run_sha:    ${RUN_SHA:-unknown}"
  echo "conclusion: ${CONCLUSION:-unknown}"
  echo "all_jobs:   ${ALL_JOBS_OK:-unknown}"
  echo "url:        $URL"
} | tee "$OUT_DIR/summary.txt"

# Защита: run обязан принадлежать ИМЕННО текущему HEAD — иначе мы смотрим чужой (конкурентный) прогон,
# и его «success» ничего не говорит про наш commit.
if [ "$RUN_SHA" != "$SHA" ]; then
  echo "VERDICT: RED — run #$RUN_ID отслеживает ${RUN_SHA:0:8}, а HEAD = ${SHA:0:8} (не тот прогон)" \
    | tee -a "$OUT_DIR/summary.txt"
  exit 2
fi

# Зелёный = агрегат success И каждый джоб success. ВЕРДИКТ — ПОСЛЕДНЯЯ строка stdout, поэтому переживает
# `ci-report.sh | tail` (где exit-код теряется): надёжно грепается как "VERDICT: GREEN".
if [ "$CONCLUSION" = "success" ] && [ "$ALL_JOBS_OK" = "true" ]; then
  echo "VERDICT: GREEN — все джобы success (run #$RUN_ID, ${SHA:0:8})" | tee -a "$OUT_DIR/summary.txt"
  exit 0
fi

# Причина: сырой лог только упавших шагов — самое ценное для LLM-агента
echo "→ Падения. Сохраняю логи упавших шагов в $OUT_DIR/failed.log"
gh run view "$RUN_ID" --log-failed > "$OUT_DIR/failed.log" 2>/dev/null || true

echo "─── последние строки лога падений ───"
tail -n 60 "$OUT_DIR/failed.log" 2>/dev/null || echo "(лог пуст)"
echo "VERDICT: RED — conclusion=${CONCLUSION:-unknown}, all_jobs=${ALL_JOBS_OK:-unknown} (run #$RUN_ID)" \
  | tee -a "$OUT_DIR/summary.txt"
exit 1
