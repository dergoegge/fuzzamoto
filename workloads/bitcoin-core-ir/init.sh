#!/bin/sh

# This script is executed inside the chroot by the Nyx fuzzer

set -e

# Set environment variables for the nyx agent
export __AFL_DEFER_FORKSRV=1
export RUST_LOG=debug
export ASAN_OPTIONS="detect_leaks=1:detect_stack_use_after_return=1:check_initialization_order=1:strict_init_order=1:log_path=/tmp/asan.log:abort_on_error=1:handle_abort=1"

# Run the IR scenario with bitcoind
/scenario-ir /bitcoind.sh > init.log 2>&1
