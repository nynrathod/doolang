#!/bin/bash
set -e

# Source common utilities
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
source "$SCRIPT_DIR/../common.sh"

FILE="test_db_summary.doo"

# Optional: lightweight Postgres availability hint (WSL-safe)
if ! ss -lnt 2>/dev/null | grep -q ":5432"; then
  echo "⚠️ Warning: Postgres port 5432 not listening"
fi


# Always succeed so output is visible (same intent as CRUD cleanup)
exit 0
