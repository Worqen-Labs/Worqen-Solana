# Worqen Escrow — developer convenience targets.
# See README.md for full docs.

PROGRAM_ID := 6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ
DEVNET_WALLET := ~/.config/solana/devnet-escrow.json
REPO_URL := https://github.com/Worqen-Labs/Worqen-Solana

.PHONY: build test fmt clippy lint deploy-devnet idl-devnet verify-devnet \
	config-status pause unpause bootstrap-config clean

## Build the program + IDL
build:
	anchor build

## Run the LiteSVM test suite in-process (no validator, supports clock warp)
test:
	anchor build && bun test

fmt:
	cargo fmt --all

clippy:
	cargo clippy --all-targets -- -D warnings

lint: fmt clippy

## Upgrade the devnet program (same program id) + on-chain IDL
deploy-devnet:
	anchor deploy --provider.cluster devnet --provider.wallet $(DEVNET_WALLET)

## (Re)publish the on-chain IDL to devnet
idl-devnet:
	anchor idl upgrade $(PROGRAM_ID) -f target/idl/worqen_escrow.json \
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
