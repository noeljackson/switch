BINARY_NAME=switch
BUILD_DIR=./build
TARGET_DIR=./target/release
VERSION=$(shell git describe --tags --always --dirty 2>/dev/null || echo "dev")

# Stamps the build; build.rs falls back to `git describe` when this is unset.
export SWITCH_VERSION=$(VERSION)

GREEN=\033[0;32m
BLUE=\033[0;34m
NC=\033[0m

.PHONY: build install install-user uninstall test fmt vet lint dev run quick update

build:
	@mkdir -p $(BUILD_DIR)
	@cargo build --release
	@cp $(TARGET_DIR)/$(BINARY_NAME) $(BUILD_DIR)/$(BINARY_NAME)

install: build
	@sudo cp $(BUILD_DIR)/$(BINARY_NAME) /usr/local/bin/
	@sudo chmod +x /usr/local/bin/$(BINARY_NAME)

install-user: build
	@mkdir -p ~/bin
	@cp $(BUILD_DIR)/$(BINARY_NAME) ~/bin/
	@chmod +x ~/bin/$(BINARY_NAME)

uninstall:
	@sudo rm -f /usr/local/bin/$(BINARY_NAME)
	@rm -f ~/bin/$(BINARY_NAME)

test:
	@cargo test

fmt:
	@cargo fmt

vet:
	@cargo clippy --all-targets -- -D warnings

lint: vet

dev: fmt vet test build

run: build
	@$(BUILD_DIR)/$(BINARY_NAME)

quick: build install-user

update:
	@echo "$(BLUE)Updating $(BINARY_NAME)...$(NC)"
	@rm -rf $(BUILD_DIR)
	@$(MAKE) build
	@sudo cp $(BUILD_DIR)/$(BINARY_NAME) /usr/local/bin/
	@sudo chmod +x /usr/local/bin/$(BINARY_NAME)
	@echo "$(GREEN)Updated $(BINARY_NAME) to $(VERSION)$(NC)"
