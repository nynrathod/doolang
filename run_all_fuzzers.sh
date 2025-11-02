#!/bin/bash

# Run all fuzzers simultaneously for 1 hour
# This script runs all 5 fuzz targets in parallel and monitors them

set -e

echo "=========================================="
echo "Starting all 5 fuzz targets for 1 hour each..."
echo "=========================================="
echo ""

# Create log directory
mkdir -p fuzz_logs

# Get start time
START_TIME=$(date +%s)
echo "Start time: $(date)"
echo ""

# Start all fuzzers in background
echo "Starting fuzz_lexer..."
cargo +nightly fuzz run fuzz_lexer -- -max_total_time=3600 -rss_limit_mb=2048 > fuzz_logs/fuzz_lexer.log 2>&1 &
LEXER_PID=$!

echo "Starting fuzz_parser..."
cargo +nightly fuzz run fuzz_parser -- -max_total_time=3600 -rss_limit_mb=2048 > fuzz_logs/fuzz_parser.log 2>&1 &
PARSER_PID=$!

echo "Starting fuzz_analyzer..."
cargo +nightly fuzz run fuzz_analyzer -- -max_total_time=3600 -rss_limit_mb=2048 > fuzz_logs/fuzz_analyzer.log 2>&1 &
ANALYZER_PID=$!

echo "Starting fuzz_mir..."
cargo +nightly fuzz run fuzz_mir -- -max_total_time=3600 -rss_limit_mb=2048 > fuzz_logs/fuzz_mir.log 2>&1 &
MIR_PID=$!

echo "Starting fuzz_codegen..."
cargo +nightly fuzz run fuzz_codegen -- -max_total_time=3600 -rss_limit_mb=2048 > fuzz_logs/fuzz_codegen.log 2>&1 &
CODEGEN_PID=$!

echo ""
echo "All 5 fuzzers started!"
echo "PIDs: Lexer=$LEXER_PID Parser=$PARSER_PID Analyzer=$ANALYZER_PID MIR=$MIR_PID Codegen=$CODEGEN_PID"
echo ""
echo "Monitoring fuzzer progress (press Ctrl+C to stop)..."
echo "You can check logs in real-time with:"
echo "  tail -f fuzz_logs/fuzz_lexer.log"
echo ""

# Trap Ctrl+C to kill all background jobs
trap 'echo ""; echo "Stopping all fuzzers..."; kill $LEXER_PID $PARSER_PID $ANALYZER_PID $MIR_PID $CODEGEN_PID 2>/dev/null; exit 1' INT

# Monitor jobs
while true; do
    sleep 60

    CURRENT_TIME=$(date +%s)
    ELAPSED=$((CURRENT_TIME - START_TIME))
    ELAPSED_MIN=$((ELAPSED / 60))

    # Count running processes
    RUNNING=0
    for PID in $LEXER_PID $PARSER_PID $ANALYZER_PID $MIR_PID $CODEGEN_PID; do
        if kill -0 $PID 2>/dev/null; then
            RUNNING=$((RUNNING + 1))
        fi
    done

    echo "[$(date +%H:%M:%S)] Elapsed: ${ELAPSED_MIN}m | Running: ${RUNNING}/5"

    # Check for errors in logs
    for LOG in fuzz_logs/*.log; do
        if [ -f "$LOG" ]; then
            if tail -5 "$LOG" | grep -q -E "ERROR|CRASH|OOM|stack-overflow"; then
                echo "  ⚠️  $(basename $LOG .log): Possible issue detected!"
            fi
        fi
    done

    # Check if all jobs finished
    if [ $RUNNING -eq 0 ]; then
        echo ""
        echo "All fuzzers have finished!"
        break
    fi

    # Safety timeout (65 minutes)
    if [ $ELAPSED -gt 3900 ]; then
        echo ""
        echo "Timeout reached (65 minutes), stopping..."
        kill $LEXER_PID $PARSER_PID $ANALYZER_PID $MIR_PID $CODEGEN_PID 2>/dev/null
        break
    fi
done

# Wait for all processes to finish
wait 2>/dev/null

echo ""
echo "=========================================="
echo "FUZZING RESULTS"
echo "=========================================="

ALL_PASSED=true

# Check each fuzzer result
for TARGET in lexer parser analyzer mir codegen; do
    LOG_FILE="fuzz_logs/fuzz_${TARGET}.log"

    echo ""
    echo "--- fuzz_${TARGET} ---"

    if [ ! -f "$LOG_FILE" ]; then
        echo "Result: ❌ FAILED - No log file found"
        ALL_PASSED=false
        continue
    fi

    # Check for crashes
    if grep -q -E "CRASH|ERROR|stack-overflow|out-of-memory|OOM" "$LOG_FILE"; then
        echo "Result: ❌ FAILED - Crash detected"
        echo "Check log: $LOG_FILE"
        echo ""
        echo "Last 20 lines of log:"
        tail -20 "$LOG_FILE"
        ALL_PASSED=false
    elif grep -q "Done .* runs" "$LOG_FILE"; then
        echo "Result: ✅ PASSED - Completed successfully"

        # Extract run count
        STATS=$(grep "Done .* runs" "$LOG_FILE" | tail -1)
        echo "Stats: $STATS"
    else
        echo "Result: ⚠️  UNKNOWN - Check log for details"
        ALL_PASSED=false
    fi
done

# Check for new artifacts
echo ""
echo "--- New Crash Artifacts ---"
NEW_ARTIFACTS=false

for DIR in fuzz_lexer fuzz_parser fuzz_analyzer fuzz_mir fuzz_codegen; do
    ARTIFACT_PATH="fuzz/artifacts/$DIR"
    if [ -d "$ARTIFACT_PATH" ]; then
        # Find files created since start time
        NEW_FILES=$(find "$ARTIFACT_PATH" -type f -newermt "@$START_TIME" 2>/dev/null)
        if [ -n "$NEW_FILES" ]; then
            COUNT=$(echo "$NEW_FILES" | wc -l)
            echo ""
            echo "$DIR : $COUNT new artifact(s)"
            echo "$NEW_FILES" | sed 's/^/  - /'
            NEW_ARTIFACTS=true
        fi
    fi
done

if [ "$NEW_ARTIFACTS" = false ]; then
    echo "No new crash artifacts found ✅"
fi

# Summary
END_TIME=$(date +%s)
TOTAL_ELAPSED=$((END_TIME - START_TIME))
TOTAL_MIN=$((TOTAL_ELAPSED / 60))

echo ""
echo "=========================================="
echo "SUMMARY"
echo "=========================================="
echo "Start time: $(date -d @$START_TIME)"
echo "End time: $(date -d @$END_TIME)"
echo "Total elapsed: ${TOTAL_MIN} minutes"

if [ "$ALL_PASSED" = true ] && [ "$NEW_ARTIFACTS" = false ]; then
    echo ""
    echo "🎉 ALL FUZZERS PASSED! 🎉"
    echo "Your compiler is production-ready!"
    exit 0
else
    echo ""
    echo "⚠️  SOME FUZZERS FAILED"
    echo "Check logs in fuzz_logs/ directory"
    echo ""
    echo "To reproduce failures:"
    echo "  cargo fuzz run fuzz_<target> <artifact_path>"
    exit 1
fi
