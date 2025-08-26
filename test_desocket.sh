#!/bin/bash

# Test script for Fuzzamoto Desocketing Implementation
echo "Testing Fuzzamoto Desocketing Implementation..."
echo "================================================"

# Test 1: Feature flag compilation
echo "1. Testing feature flag compilation..."
if cargo check --package fuzzamoto --features desocket; then
    echo "   ✅ Feature flag compilation successful"
else
    echo "   ❌ Feature flag compilation failed"
    exit 1
fi

# Test 2: Default compilation (backward compatibility)
echo "2. Testing default compilation..."
if cargo check --package fuzzamoto; then
    echo "   ✅ Default compilation successful"
else
    echo "   ❌ Default compilation failed"
    exit 1
fi

# Test 3: Unit tests with desocket feature
echo "3. Testing unit tests with desocket feature..."
if cargo test --package fuzzamoto --features desocket; then
    echo "   ✅ Desocket tests passed"
else
    echo "   ❌ Desocket tests failed"
    exit 1
fi

# Test 4: Unit tests without desocket feature
echo "4. Testing unit tests without desocket feature..."
if cargo test --package fuzzamoto; then
    echo "   ✅ Default tests passed"
else
    echo "   ❌ Default tests failed"
    exit 1
fi

echo ""
echo "🎉 All tests passed! Desocketing foundation is working correctly."
echo ""
echo "Next steps:"
echo "- Add target integration"
echo "- Implement libdesock integration"
echo "- Add performance benchmarking"
