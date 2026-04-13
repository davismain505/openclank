#!/bin/bash
set -euo pipefail

HOST="root@omnios-big.local"

ssh "${HOST}" "mkdir -p ~/.claude"
scp ~/.claude/.credentials.json "${HOST}:~/.claude/.credentials.json"
ssh "${HOST}" "chmod 600 ~/.claude/.credentials.json"
