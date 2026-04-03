#!/usr/bin/env bash
# Example hook script for the harness plugin system.
#
# This script is called by the harness when a lifecycle event fires.
# Available environment variables:
#   HARNESS_HOOK        - which hook is firing
#   HARNESS_PLUGIN      - plugin name
#   HARNESS_PROJECT     - project directory name
#   HARNESS_DIR         - path to .harness/ directory
#   HARNESS_PLUGINS_DIR - path to plugins directory

set -euo pipefail

LOG_FILE="${HARNESS_DIR}/plugin-log.txt"

echo "[$(date '+%Y-%m-%d %H:%M:%S')] ${HARNESS_PLUGIN}: ${HARNESS_HOOK} fired for ${HARNESS_PROJECT}" >> "$LOG_FILE"

# Example: run tests after build
if [ "$HARNESS_HOOK" = "after_build" ]; then
    echo "Running post-build checks..."
    if command -v cargo &>/dev/null && [ -f Cargo.toml ]; then
        cargo check 2>&1 || echo "cargo check failed"
    fi
fi
