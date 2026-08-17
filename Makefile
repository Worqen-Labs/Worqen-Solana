# Worqen Escrow — developer convenience targets.
# See README.md for full docs.

PROGRAM_ID := FinhtLJ4PVBwVi8tGwWoCzN3vDpcMofBZFXFzmntGxEh
DEVNET_WALLET := ~/.config/solana/devnet-escrow.json
REPO_URL := https://github.com/Worqen-Labs/Worqen-Solana
MANIFEST := programs/worqen-escrow/Cargo.toml

# Two artifacts, on purpose. `anchor build` emits the default-arch (SBPFv0)
# .so that LiteSVM 1.1.0 can load, plus the IDL; `build-deploy` emits the
# SBPFv3 .so that every cluster deploy ships. See "Two build artifacts" in
# CLAUDE.md for why they cannot be the same file.
SBF_ARCH := v3
SBF_TOOLS := v1.55
DEPLOY_DIR := target/deploy-v3
DEPLOY_SO := $(DEPLOY_DIR)/worqen_escrow.so
TEST_SO := target/deploy/worqen_escrow.so
# ProgramData allocation of the deployed mainnet program, in bytes.
MAINNET_ALLOCATION := 700040

.PHONY: build build-deploy sizes test fmt clippy lint deploy-devnet idl-devnet \
	verify-devnet config-status pause unpause bootstrap-config clean

## Build the test/IDL artifact: default-arch .so (LiteSVM-loadable) + the IDL
build:
	anchor build

## Build the DEPLOYABLE artifact: SBPFv3 .so, ~154 KB smaller than `build`'s.
## Anchor's CLI pins its own platform-tools, so the arch flags cannot go through
## `anchor build` — the deployable goes through cargo build-sbf directly and
## anchor build is kept only for the IDL. Needs Agave >= 4.0.0 to deploy.
##   make build-deploy SBF_FEATURES="--features mainnet"
build-deploy:
	cargo build-sbf --manifest-path $(MANIFEST) --arch $(SBF_ARCH) \
		--tools-version $(SBF_TOOLS) --sbf-out-dir $(DEPLOY_DIR) $(SBF_FEATURES)

## Print both artifact sizes against the mainnet ProgramData allocation
sizes: build build-deploy
	@printf '%-28s %9d B\n' "default-arch (tests)" $$(stat -c '%s' $(TEST_SO))
	@printf '%-28s %9d B\n' "$(SBF_ARCH) (deploys)" $$(stat -c '%s' $(DEPLOY_SO))
	@printf '%-28s %9d B\n' "mainnet allocation" $(MAINNET_ALLOCATION)
	@printf '%-28s %9d B\n' "extend needed ($(SBF_ARCH))" \
		$$(( $$(stat -c '%s' $(DEPLOY_SO)) + 45 - $(MAINNET_ALLOCATION) ))

## Run the LiteSVM test suite in-process (no validator, supports clock warp)
test:
	anchor build && bun test

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets -- -D warnings

lint: fmt clippy

## Upgrade the devnet program (same program id) with the SBPFv3 artifact.
## The Agave CLI refuses a v3 upload against a cluster that has not activated
## SIMD-0377, so a stale cluster fails here instead of bricking the program.
deploy-devnet: build-deploy
	anchor upgrade $(DEPLOY_SO) --program-id $(PROGRAM_ID) \
		--provider.cluster devnet --provider.wallet $(DEVNET_WALLET)

## (Re)publish the on-chain IDL to devnet (Anchor 1.x: Program Metadata Program;
## `init` covers the first publish, `upgrade` every one after)
idl-devnet:
	anchor idl upgrade $(PROGRAM_ID) -f target/idl/worqen_escrow.json \
		--provider.cluster devnet --provider.wallet $(DEVNET_WALLET) \
	|| anchor idl init $(PROGRAM_ID) -f target/idl/worqen_escrow.json \
		--provider.cluster devnet --provider.wallet $(DEVNET_WALLET)

## Reproducible verified build + on-chain verification (needs Docker + a public repo).
## Run after the repo is pushed; the deployed artifact must be the reproducible one.
verify-devnet:
	./scripts/verify.sh devnet $(PROGRAM_ID) $(REPO_URL)

## Print the on-chain Config: paused flag, authority, treasury, allowlist. Read-only.
##   make config-status RPC_URL=https://...
config-status:
	RPC_URL=$(RPC_URL) bun scripts/pause.ts status

## EMERGENCY kill-switch: block new escrows (release/dispute/close stay open).
##   make pause RPC_URL=https://... AUTHORITY_KEYPAIR=~/key.json
pause:
	RPC_URL=$(RPC_URL) AUTHORITY_KEYPAIR=$(AUTHORITY_KEYPAIR) bun scripts/pause.ts pause

## Resume after a pause.
##   make unpause RPC_URL=https://... AUTHORITY_KEYPAIR=~/key.json
unpause:
	RPC_URL=$(RPC_URL) AUTHORITY_KEYPAIR=$(AUTHORITY_KEYPAIR) bun scripts/pause.ts unpause

## One-time Config init / mint-allowlist reconcile (idempotent).
##   make bootstrap-config RPC_URL=https://... AUTHORITY_KEYPAIR=~/key.json \
##     FEE_RECIPIENT=<treasury> ALLOWED_MINTS=<usdc>,<usdt>,<eurc>
bootstrap-config:
	RPC_URL=$(RPC_URL) AUTHORITY_KEYPAIR=$(AUTHORITY_KEYPAIR) FEE_RECIPIENT=$(FEE_RECIPIENT) \
		DEFAULT_BPS=$(or $(DEFAULT_BPS),500) ALLOWED_MINTS=$(ALLOWED_MINTS) \
		bun scripts/bootstrap-config.ts

clean:
	anchor clean
