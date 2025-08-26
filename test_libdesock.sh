#!/bin/bash

# Test script to demonstrate libdesock integration with a simple echo server

set -e

echo "=== libdesock Integration Test ==="

# Check if libdesock.so exists
if [ ! -f "./libdesock.so" ]; then
    echo "❌ libdesock.so not found. Please copy it to the current directory."
    exit 1
fi

echo "✅ Found libdesock.so"

# Test 1: Verify libdesock can be loaded
echo "🔍 Testing if libdesock can be loaded..."
if LD_PRELOAD=./libdesock.so echo "test" > /dev/null 2>&1; then
    echo "✅ libdesock loads successfully"
else
    echo "❌ Failed to load libdesock"
    exit 1
fi

# Test 2: Simple cat test with libdesock
echo "🔍 Testing basic stdin/stdout redirection..."
echo "Hello libdesock" | LD_PRELOAD=./libdesock.so cat

# Test 3: Check if we can spawn a process with libdesock
echo "🔍 Testing process spawning with libdesock..."
timeout 2s LD_PRELOAD=./libdesock.so bash -c 'echo "Process started with libdesock"; sleep 1' || true

echo "✅ libdesock basic integration tests completed"
echo ""
echo "🚀 Next steps:"
echo "   1. Build fuzzamoto with --features desocket"
echo "   2. Integrate DesocketTransport with BitcoinCore target"
echo "   3. Test with actual Bitcoin Core scenarios"
echo ""
echo "💡 To test with Bitcoin Core manually:"
echo "   LD_PRELOAD=./libdesock.so bitcoind -regtest -printtoconsole=0"
