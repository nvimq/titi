#!/usr/bin/env bash
# Валидатор структуры ресерч-фундамента titi. Exit 0 = полнота.
set -u
cd "$(dirname "$0")/.." || exit 1
fail=0

theme_dir() { echo "docs/research/$1"; }

THEMES=(config-settings secrets-env sessions-persistence compaction-context
system-prompt-soul memory-learning trajectory-gepa providers-streaming
toolconv tools-core tools-advanced-cua model-switching tui-renderer
agent-ux extensibility-marketplace mcp agents-hub-security bot-network-soul
gui-gpui packaging-headless)

SECTIONS='^# .+|^## omp|^## Hermes|^## Vellum|^## Решение \(одно/комбо\)|^## Rust-маппинг|^## Definition of Done|^## Deep-dive'

check_doc() {
  local file="$1"
  if [ ! -f "$file" ]; then echo "MISSING file: $file"; fail=1; return; fi
  for sec in "^# " "^## omp$" "^## Hermes$" "^## Vellum$" '^## Решение \(одно/комбо\)$' "^## Rust-маппинг$" "^## Definition of Done$" "^## Deep-dive$"; do
    if ! grep -qE "$sec" "$file"; then echo "MISSING section '$sec' in $file"; fail=1; fi
  done
}
grep -q "графов" docs/CONVEYOR.md 2>/dev/null || { echo "CONVEYOR: нет граф-практик"; fail=1; }
# 1. Корневые артефакты
for f in docs/research/README.md docs/PLAN.md docs/CONVEYOR.md docs/research/STATE.md scripts/check-research.sh; do
  [ -f "$f" ] || { echo "MISSING: $f"; fail=1; }
done
grep -q "Граф зависимостей" docs/research/README.md 2>/dev/null || { echo "README: нет графа зависимостей"; fail=1; }
grep -q "Definition of Done\|DoD" docs/PLAN.md 2>/dev/null || { echo "PLAN: нет DoD"; fail=1; }
grep -q "in-progress\|done\|todo" docs/research/STATE.md 2>/dev/null || { echo "STATE: нет статусов"; fail=1; }

# 2. Темы и секции
for t in "${THEMES[@]}"; do
  check_doc "$(theme_dir "$t")/README.md"
done

# 3. Deep-dive 2-го уровня: минимум 3 подсистемы в трёх темах
for t in memory-learning providers-streaming tui-renderer; do
  count=$(find "$(theme_dir "$t")" -name '*.md' ! -name 'README.md' 2>/dev/null | wc -l | tr -d ' ')
  if [ "$count" -lt 3 ]; then echo "DEEP: $t имеет $count подсистем (нужно >=3)"; fail=1; else
    for sub in "$(theme_dir "$t")"/*.md; do
      [ "$(basename "$sub")" = "README.md" ] && continue
      check_doc "$sub"
    done
  fi
done

if [ "$fail" -eq 0 ]; then echo "OK: research structure complete (${#THEMES[@]} themes + 3 deep-dive)"; fi
exit $fail
