#!/bin/bash


# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Define binary path (WSL/Linux only)
BIN="$SCRIPT_DIR/../../target/release/doo"

# Check if binary exists
if [ ! -f "$BIN" ]; then
    echo "Error: Binary not found at $BIN"
    exit 1
fi

# Find all .doo files excluding database and http directories
# Sort them to ensure deterministic run order
find "$SCRIPT_DIR" -type f -name "*.doo" \
    -not -path "*/database/*" \
    -not -path "*/http/*" \
    | sort | while read -r file; do
    
    # Print relative path for better readability
    rel_path="${file#$SCRIPT_DIR/}"
    echo "Running: $rel_path"
    
    # Run the test
    "$BIN" run "$file"
done
