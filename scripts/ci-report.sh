#!/usr/bin/env bash
# Запускать ПОСЛЕ git push. Находит CI-run для текущего HEAD, ждёт его и
# собирает машиночитаемый отчёт для агента.
# Exit code: 0 = success, 1 = тесты упали, 2 = ошибка инфраструктуры/таймаут.
set -uo pipefail

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

# Метаданные прогона
gh run view "$RUN_ID" \
  --json status,conclusion,displayTitle,headSha,url,jobs \
  > "$OUT_DIR/run.json" 2>/dev/null
CONCLUSION="$(gh run view "$RUN_ID" --json conclusion --jq .conclusion 2>/dev/null)"
URL="$(gh run view "$RUN_ID" --json url --jq .url 2>/dev/null)"

# Структура: какие тесты упали (JUnit XML). Два джоба (x86 + arm64) кладут по своему артефакту
# (test-report-x86 / test-report-arm64) — тянем ВСЕ в .ci-report/artifacts/<name>/junit.xml.
gh run download "$RUN_ID" -D "$OUT_DIR/artifacts" 2>/dev/null || true

{
  echo "commit:     $SHA"
  echo "run:        #$RUN_ID"
  echo "conclusion: ${CONCLUSION:-unknown}"
  echo "url:        $URL"
} | tee "$OUT_DIR/summary.txt"

if [ "$CONCLUSION" = "success" ]; then
  echo "✓ Все тесты прошли."
  exit 0
fi

# Причина: сырой лог только упавших шагов — самое ценное для LLM-агента
echo "→ Падения. Сохраняю логи упавших шагов в $OUT_DIR/failed.log"
gh run view "$RUN_ID" --log-failed > "$OUT_DIR/failed.log" 2>/dev/null || true

echo "─── последние строки лога падений ───"
tail -n 60 "$OUT_DIR/failed.log" 2>/dev/null || echo "(лог пуст)"
exit 1
