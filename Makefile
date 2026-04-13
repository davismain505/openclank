HOST := root@omnios-big.local
REMOTE_DIR := ~/openclank

.PHONY: test test-live test-illumos test-illumos-live sync sync-credentials

test:
	cargo test

test-live:
	cargo test --test live_api_tests -- --ignored

sync:
	rsync -az --exclude target/ --exclude .git/ ./ $(HOST):$(REMOTE_DIR)/

sync-credentials:
	ssh $(HOST) "mkdir -p ~/.claude"
	scp ~/.claude/.credentials.json $(HOST):~/.claude/.credentials.json
	ssh $(HOST) "chmod 600 ~/.claude/.credentials.json"

test-illumos: sync
	ssh $(HOST) 'source ~/.zshrc && cd $(REMOTE_DIR) && gmake test'

test-illumos-live: sync sync-credentials
	ssh $(HOST) 'source ~/.zshrc && cd $(REMOTE_DIR) && gmake test-live'
