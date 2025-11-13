#!/bin/sh
LD_PRELOAD=/libnyx_crash_handler.so /bitcoind $@
