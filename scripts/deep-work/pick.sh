#!/usr/bin/env bash
# pick.sh
#
# Smart issue selection based on workflow criteria:
# - Filter by complexity (easy to medium)
# - Filter by description quality (substantial content)
# - Prioritize bugs over features
# - Avoid blocked issues

set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=./lib.sh
source "$here/lib.sh"

require gh jq

repo=$(resolve_deep_work_repo)

echo "[deep-work] 🔍 Smart issue picking from $repo..."

# Get all open issues
echo "[deep-work] fetching open issues..."
issues_json=$(gh issue list -R "$repo" --state open --limit 100 \
  --json number,title,body,labels,assignees,state,url)

if [ "$(echo "$issues_json" | jq 'length')" -eq 0 ]; then
  echo "[deep-work] no open issues found in $repo"
  exit 1
fi

echo "[deep-work] found $(echo "$issues_json" | jq 'length') open issues"

# Filter and score issues
echo "[deep-work] analyzing issues..."

candidates=$(echo "$issues_json" | jq -r '
  map(select(
    # Must be open
    .state == "OPEN" and

    # Must not be assigned (or ask user about assigned ones)
    (.assignees | length == 0) and

    # Must have substantial description (>500 chars)
    (.body // "" | length > 500)
  )) |

  # Add scoring
  map(
    . + {
      "score": (
        # Prefer bugs (+3)
        (if (.labels | map(.name) | contains(["bug"])) then 3 else 0 end) +

        # Prefer enhancement (+2)
        (if (.labels | map(.name) | contains(["enhancement"])) then 2 else 0 end) +

        # Prefer good-first-issue (+1)
        (if (.labels | map(.name) | contains(["good first issue"])) then 1 else 0 end) +

        # Penalize large scope labels (-2)
        (if (.labels | map(.name) | contains(["epic", "large", "major"])) then -2 else 0 end) +

        # Description quality bonus (length / 1000)
        ((.body // "" | length) / 1000 | floor)
      ),
      "complexity": (
        if (.labels | map(.name) | contains(["epic", "large", "major"])) then "hard"
        elif (.labels | map(.name) | contains(["good first issue", "easy"])) then "easy"
        else "medium"
        end
      )
    }
  ) |

  # Filter out hard complexity
  map(select(.complexity != "hard")) |

  # Sort by score (descending)
  sort_by(-.score) |

  # Take top 10
  .[0:10]
')

num_candidates=$(echo "$candidates" | jq 'length')

if [ "$num_candidates" -eq 0 ]; then
  echo "[deep-work] no suitable issues found after filtering"
  echo ""
  echo "Criteria used:"
  echo "- Open and unassigned"
  echo "- Substantial description (>500 chars)"
  echo "- Not labeled as epic/large/major complexity"
  echo ""
  echo "Try browsing issues manually: gh issue list -R $repo"
  exit 1
fi

echo "[deep-work] found $num_candidates suitable candidates"
echo ""

# Display top candidates
echo "🏆 Top issue candidates:"
echo ""

echo "$candidates" | jq -r '.[] |
  "\(.number). \(.title)
  🏷️  Labels: \((.labels | map(.name) | join(", ")) // "none")
  📊 Score: \(.score) | Complexity: \(.complexity)
  📝 Description: \(.body // "no description" | .[0:100])...
  🔗 \(.url)
  "
'

echo ""
echo "Select an issue to work on:"
echo "1-$num_candidates) Pick by number from list above"
echo "0) Browse all issues manually"
echo "q) Quit"
echo ""

while true; do
  read -p "Your choice [1-$num_candidates/0/q]: " -r choice

  case "$choice" in
    q|Q)
      echo "Cancelled."
      exit 0
      ;;
    0)
      echo "[deep-work] opening issue browser..."
      gh issue list -R "$repo" --web
      exit 0
      ;;
    ''|*[!0-9]*)
      echo "Please enter a number between 1-$num_candidates, 0, or q"
      continue
      ;;
    *)
      if [ "$choice" -ge 1 ] && [ "$choice" -le "$num_candidates" ]; then
        selected_idx=$((choice - 1))
        selected_issue=$(echo "$candidates" | jq -r ".[$selected_idx].number")
        selected_title=$(echo "$candidates" | jq -r ".[$selected_idx].title")

        echo ""
        echo "[deep-work] 🎯 Selected issue #$selected_issue: $selected_title"
        echo ""
        echo "Starting full workflow..."
        exec "$here/start.sh" "$selected_issue"
      else
        echo "Please enter a number between 1-$num_candidates, 0, or q"
        continue
      fi
      ;;
  esac
done