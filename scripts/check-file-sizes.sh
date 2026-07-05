#!/usr/bin/env bash
# Module-size ratchet — hand-written source must trend toward small, focused
# files. Existing oversized files are baselined in scripts/.god-object-baseline.txt
# with their current LOC as a ceiling; they may only SHRINK. A NEW file over the
# threshold, or GROWTH of a baselined file past its ceiling, fails.
#
# Doctrine: CLAUDE.md "Module Size Discipline". Split god objects by
# responsibility; the baseline exists to be driven to zero, not appeased.
#
#   scripts/check-file-sizes.sh            # enforce (used by the pre-commit hook + CI)
#   scripts/check-file-sizes.sh --update   # re-record ceilings after a real split (records shrinks only)
set -euo pipefail

THRESHOLD=600
cd "$(git rev-parse --show-toplevel)"
BASELINE="scripts/.god-object-baseline.txt"

# Generated / vendored / test files are exempt — the rule is about hand-written
# modules a human must read and reason about.
is_exempt() {
  case "$1" in
    *node_modules/*|*/dist/*|*.test.*|*/tests/*|*abi.ts|*.gen.*|*generated*) return 0 ;;
    *) return 1 ;;
  esac
}

sources() { git ls-files '*.rs' '*.ts' '*.tsx'; }

if [ "${1:-}" = "--update" ]; then
  {
    echo "# God-object baseline (LOC ceilings). Files here may only SHRINK."
    echo "# Regenerate after a real split: scripts/check-file-sizes.sh --update"
    echo "# Doctrine: CLAUDE.md 'Module Size Discipline' — split by responsibility toward <${THRESHOLD} LOC."
    while read -r f; do
      is_exempt "$f" && continue
      [ -f "$f" ] || continue
      n=$(wc -l <"$f")
      [ "$n" -gt "$THRESHOLD" ] && echo "$n $f"
    done < <(sources) | sort -rn
  } >"$BASELINE"
  echo "✓ baseline updated ($(grep -cvE '^#' "$BASELINE") files over ${THRESHOLD} LOC)"
  exit 0
fi

declare -A CEIL
while read -r n f; do
  [[ "$n" =~ ^[0-9]+$ ]] && CEIL["$f"]=$n
done < <(grep -vE '^#' "$BASELINE" 2>/dev/null || true)

fail=0
while read -r f; do
  is_exempt "$f" && continue
  [ -f "$f" ] || continue
  n=$(wc -l <"$f")
  [ "$n" -le "$THRESHOLD" ] && continue
  ceil="${CEIL[$f]:-}"
  if [ -z "$ceil" ]; then
    echo "❌ NEW god object: $f is $n LOC (> $THRESHOLD). Split it by responsibility (see CLAUDE.md)."
    fail=1
  elif [ "$n" -gt "$ceil" ]; then
    echo "❌ god object GREW: $f is $n LOC (baseline ceiling $ceil). Baselined files may only shrink — split it."
    fail=1
  fi
done < <(sources)

if [ "$fail" -ne 0 ]; then
  echo ""
  echo "Module-size ratchet failed. The baseline is a debt to pay down, never grow."
  exit 1
fi
echo "✓ module-size ratchet: no new or grown god objects"
