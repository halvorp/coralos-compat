#!/bin/sh
# start.sh : boot AROS m68k (A1200, 8M fast) under Copperline headless, running the launcher
cd /data/build/oracle/dopus; rm -f ccp.json
/data/build/copperline/release/copperline --config dopus.toml --noaudio --model A1200 --fast 8M --run /data/build/oracle/dopus/launch --control 127.0.0.1:0 --control-info ccp.json > cl.out 2>&1 &
while [ ! -s ccp.json ]; do sleep 0.3; done
