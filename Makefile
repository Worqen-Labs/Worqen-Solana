# Worqen Escrow — developer convenience targets.
# See README.md for full docs.

PROGRAM_ID := 6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ
DEVNET_WALLET := ~/.config/solana/devnet-escrow.json
REPO_URL := https://github.com/Worqen-Labs/Worqen-Escrow

.PHONY: build test fmt clippy lint deploy-devnet idl-devnet verify-devnet clean

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

clean:
	anchor clean
