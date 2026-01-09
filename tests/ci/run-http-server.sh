#!/bin/sh

mkdir /tmp/in && echo "AAA" > /tmp/in/A
timeout 60s /AFLplusplus/afl-fuzz -X -i /tmp/in -o /tmp/out -- /tmp/fuzzamoto_scenario-http-server
