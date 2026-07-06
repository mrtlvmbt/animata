#!/usr/bin/env bash
# done-gate — Stop-хук кодеров animata (план animata-followup 2026-07-06, Шаг 2; F1/F5/F6/F7-фиксы
# консенсуса). Гейтит ТОЛЬКО финальный отчёт с литеральной строкой `STATUS: done`: прогоняет
# done-check.sh и блокирует «done» без PR/CI/ТЗ (класс A-4). Промежуточные ходы («шаг сделан, иду
# дальше») и ходы без STATUS-строки не трогает. Порядок строго: дешёвый паттерн-матч ДО любых
# сетевых вызовов. При stop_hook_active=true повторно НЕ блокирует (исключение цикла by design),
# но перепроверяет и пишет громкий BLOCKED-OVERRIDE в .claude/done-gate.log — PM обрабатывает лог
# как hard-fail очередь приёма. Санкционированный одноразовый обход: файл .claude/.done-allow
# (создание — видимое действие; потребляется и логируется, зеркало KIT_ALLOW_DIRTY).
# Включается слотом ANIMATA_DONE_GATE=1 (kit.config.sh, роль-скоуп A/B/C). Без jq — no-op (exit 0),
# как все kit-хуки. bash 3.2-safe.
set -u

command -v jq >/dev/null 2>&1 || exit 0
PROJECT_DIR="${CLAUDE_PROJECT_DIR:-$(pwd)}"
# shellcheck disable=SC1091
[ -f "$PROJECT_DIR/.claude/kit.config.sh" ] && . "$PROJECT_DIR/.claude/kit.config.sh"
[ "${ANIMATA_DONE_GATE:-0}" = "1" ] || exit 0

INPUT="$(cat)"
STOP_ACTIVE="$(printf '%s' "$INPUT" | jq -r '.stop_hook_active // false')"
TRANSCRIPT="$(printf '%s' "$INPUT" | jq -r '.transcript_path // empty')"
[ -n "$TRANSCRIPT" ] && [ -f "$TRANSCRIPT" ] || exit 0

# Последнее assistant-сообщение (хвост транскрипта достаточен — финальный ход всегда в конце).
LAST_TXT="$(tail -n 400 "$TRANSCRIPT" | jq -c 'select(.type=="assistant")' | tail -n 1 \
  | jq -r '.message.content[]? | select(.type=="text") | .text' 2>/dev/null)"
# F5: стреляем ТОЛЬКО на литеральный терминальный токен, не на прозу.
printf '%s\n' "$LAST_TXT" | grep -q '^STATUS: done' || exit 0

LOG="$PROJECT_DIR/.claude/done-gate.log"
STAMP="$(date '+%Y-%m-%d %H:%M:%S')"

# Санкционированный одноразовый обход (видимый + логируемый + потребляемый).
if [ -f "$PROJECT_DIR/.claude/.done-allow" ]; then
  rm -f "$PROJECT_DIR/.claude/.done-allow"
  echo "$STAMP ALLOW: .done-allow consumed — done-gate пропущен сознательно" >> "$LOG"
  exit 0
fi

# Сетевые вызовы — только после матча токена (F6: ноль форков на обычных ходах).
CHECK_OUT="$(bash "$PROJECT_DIR/.claude/hooks/done-check.sh" 2>&1)" && exit 0

if [ "$STOP_ACTIVE" = "true" ]; then
  # F7: one-shot гейт — повторный блок запрещён (анти-цикл), но провал ГРОМКИЙ на стороне приёма.
  echo "$STAMP BLOCKED-OVERRIDE: STATUS: done повторён после блока, done-check всё ещё FAIL → $CHECK_OUT" >> "$LOG"
  exit 0
fi

echo "$STAMP BLOCK: STATUS: done при done-check FAIL → $CHECK_OUT" >> "$LOG"
jq -n --arg reason "done-gate: отчёт 'STATUS: done' отклонён — $CHECK_OUT
Правило: 'done' существует только с открытым PR, зелёным CI и закрытыми пунктами ТЗ.
Если работа реально не доведена до PR/CI — перепиши финальный отчёт как
'STATUS: blocked@<шаг>: <что нужно>' (это честный и разрешённый исход).
Сознательный обход (исключение): создай файл .claude/.done-allow и повтори — обход будет залогирован." \
  '{decision: "block", reason: $reason}'
exit 0
