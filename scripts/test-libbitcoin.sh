#!/bin/bash
# Test that libbitcoin-server starts and listens on P2P port
set -e

LIBBITCOIN_PATH="${LIBBITCOIN_PATH:-/opt/libbitcoin/bin/bs}"
rm -rf /tmp/bs-test-db /tmp/bs-test-logs

cat > /tmp/bs.cfg << 'EOF'
[log]
archive_directory = /tmp/bs-test-logs
debug_file = /tmp/bs-test-logs/debug.log
error_file = /tmp/bs-test-logs/error.log

[network]
identifier = 3669344250
inbound_port = 18445
inbound_connections = 10
outbound_connections = 0
host_pool_capacity = 0

[database]
directory = /tmp/bs-test-db

[blockchain]
use_libconsensus = false
EOF

$LIBBITCOIN_PATH --config /tmp/bs.cfg --initchain >/dev/null 2>&1
$LIBBITCOIN_PATH --config /tmp/bs.cfg >/dev/null 2>&1 &
BS_PID=$!
trap "kill $BS_PID 2>/dev/null; rm -rf /tmp/bs-test-db /tmp/bs-test-logs" EXIT

for i in {1..30}; do
    if (echo > /dev/tcp/127.0.0.1/18445) 2>/dev/null; then
        echo "OK: libbitcoin-server listening on port 18445"
        exit 0
    fi
    echo "Waiting for port 18445... ($i/30)"
    sleep 1
done

echo "FAIL: port 18445 not open after 30s"
exit 1
