#!/bin/bash
set -euo pipefail

HOST="root@omnios-big.local"
REMOTE_DIR="~/openclank"

rsync -az --exclude target/ --exclude .git/ \
    "$(dirname "$0")/../" "${HOST}:${REMOTE_DIR}/"

ssh "${HOST}" "source ~/.zshrc && cd ${REMOTE_DIR} && cargo test $*"
