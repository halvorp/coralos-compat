#!/bin/sh
cd /data/build/oracle/dopus; timeout 10 python3 -c "
import sys; sys.path.insert(0,'tools')
from cc import CC
CC('ccp.json').call('shutdown')" ; sleep 1
