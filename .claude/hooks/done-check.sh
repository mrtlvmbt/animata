#!/usr/bin/env bash
# done-check — машинное определение DONE для кодеров animata (план animata-followup 2026-07-06, Шаг 2).
# PASS только если: (а) для текущей ветки существует OPEN PR; (б) CI на HEAD зелёный (никакого
# эскейпа «CI ожидается» — двухпроходная задача на проходе 1 отчитывается `STATUS: blocked@N: жду CI`);
# (в) в теле PR нет незакрытых чекбоксов ТЗ (`- [ ]`).
# FAIL-CLOSED: любая ошибка самого чека (нет gh, нет auth, сеть, detached HEAD) = FAIL с причиной —
# гейт блокирует только слово «done», цена ложного блока = переформулировка отчёта в blocked@.
# Вызов: standalone (кодером до финального отчёта) или из done-gate.sh. stdout: PASS | FAIL: <причины>.
# Exit: 0 = PASS, 1 = FAIL. bash 3.2-safe.
set -u

fail() { echo "FAIL: $*"; exit 1; }

command -v gh >/dev/null 2>&1 || fail "gh не установлен — проверить PR/CI невозможно (fail-closed)"
command -v jq >/dev/null 2>&1 || fail "jq не установлен — распарсить статус CI невозможно (fail-closed)"

BRANCH="$(git branch --show-current 2>/dev/null || true)"
[ -n "$BRANCH" ] || fail "detached HEAD — работа без ветки не бывает done"
case "$BRANCH" in main|master) fail "на $BRANCH done не отчитываются — работа идёт в feature-ветке" ;; esac

PR_JSON="$(gh pr view --json number,state,body,statusCheckRollup 2>&1)" \
  || fail "PR для ветки '$BRANCH' не найден (gh: $(printf '%s' "$PR_JSON" | head -1)) — это ровно кейс A-4"

STATE="$(printf '%s' "$PR_JSON" | jq -r '.state')"
[ "$STATE" = "OPEN" ] || fail "PR в состоянии $STATE, не OPEN"

# CI: каждый check в rollup обязан быть завершён успешно (SUCCESS/NEUTRAL/SKIPPED).
BAD="$(printf '%s' "$PR_JSON" | jq -r '[.statusCheckRollup[]?
        | select((.conclusion // .state // "PENDING") | test("SUCCESS|NEUTRAL|SKIPPED") | not)
        | (.name // .context // "check")] | join(", ")')"
[ -z "$BAD" ] || fail "CI не зелёный: $BAD (ждёшь CI → отчитывайся 'STATUS: blocked@N: жду CI', не done)"
N_CHECKS="$(printf '%s' "$PR_JSON" | jq -r '[.statusCheckRollup[]?] | length')"
[ "${N_CHECKS:-0}" -gt 0 ] || fail "на HEAD нет ни одного CI-чека — прогони пайплайн (push → ci-report.sh)"

# ТЗ-пункты: незакрытый чекбокс в теле PR = недоделанный пункт.
UNCHECKED="$(printf '%s' "$PR_JSON" | jq -r '.body' | grep -c '^\s*- \[ \]' || true)"
[ "${UNCHECKED:-0}" -eq 0 ] || fail "в теле PR $UNCHECKED незакрытых пунктов ТЗ (- [ ])"

echo "PASS"
exit 0
