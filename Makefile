HOST := root@omnios-big.local
REMOTE_DIR := ~/openclank

TLA_DIR := target/tla
TLA_JAR := $(TLA_DIR)/tla2tools.jar
TLA_VERSION := 1.7.4
TLA_URL := https://github.com/tlaplus/tlaplus/releases/download/v$(TLA_VERSION)/tla2tools.jar

SPEC_DIR := spec
TLA_CHECKS := OpenClank RunnerLoop

.PHONY: test test-live test-illumos test-illumos-live sync sync-credentials check

test: test-local test-illumos

test-live: test-local-live test-illumos-live

test-local:
	cargo test

test-local-live:
	cargo test --test live_api_tests -- --ignored

sync:
	rsync -az --exclude target/ --exclude .git/ ./ $(HOST):$(REMOTE_DIR)/

sync-credentials:
	ssh $(HOST) "mkdir -p ~/.claude"
	scp ~/.claude/.credentials.json $(HOST):~/.claude/.credentials.json
	ssh $(HOST) "chmod 600 ~/.claude/.credentials.json"

test-illumos: sync
	ssh $(HOST) 'source ~/.zshrc && cd $(REMOTE_DIR) && gmake test-local'

test-illumos-live: sync sync-credentials
	ssh $(HOST) 'source ~/.zshrc && cd $(REMOTE_DIR) && gmake test-local-live'

check: $(TLA_JAR)
	@for spec in $(TLA_CHECKS); do \
		echo "=== Checking $$spec ==="; \
		java -jar $(TLA_JAR) -nowarning -metadir target/tla/states \
			-config "$(SPEC_DIR)/$${spec}Test.cfg" "$(SPEC_DIR)/$${spec}.tla" || exit 1; \
	done

$(TLA_JAR):
	@mkdir -p $(TLA_DIR)
	curl -L -o $(TLA_JAR) $(TLA_URL)
