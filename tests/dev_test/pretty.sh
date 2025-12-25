#!/bin/bash
# Wrapper to pretty print JSON if valid, otherwise print raw
if ! command -v jq &> /dev/null; then
    cat
    exit 0
fi

INPUT=$(cat)
# Try to parse with jq
if echo "$INPUT" | jq . >/dev/null 2>&1; then
    echo "$INPUT" | jq .
else
    echo "$INPUT"
fi
