#!/usr/bin/env bash
# Render one or more Ghosty compositions as transparent ProRes 4444 .mov files.
#
# Usage:
#   ./scripts/render-transparent.sh                       # renders GhostyWave by default
#   ./scripts/render-transparent.sh GhostyRecording       # renders one composition
#   ./scripts/render-transparent.sh GhostyWave GhostyIdle # renders multiple
#   pnpm render:all                                       # renders every variant
#
# Output: out/<CompositionId>.mov
set -euo pipefail

cd "$(dirname "$0")/.."
mkdir -p out

COMPS=("$@")
if [ ${#COMPS[@]} -eq 0 ]; then
  COMPS=("GhostyWave")
fi

for comp in "${COMPS[@]}"; do
  echo "▶ Rendering $comp → out/$comp.mov"
  pnpm exec remotion render "$comp" "out/$comp.mov" \
    --codec=prores \
    --prores-profile=4444 \
    --pixel-format=yuva444p10le
done

echo "✓ Done. Files in ./out/"
