#!/usr/bin/env bash
#
# Reproducible verified build + on-chain verification for Worqen Escrow.
#
# This produces the "Program is verified: True" badge on Solana explorers
# (Solscan / explorer.solana.com) and surfaces the on-chain security.txt by
# registering a reproducible build in the OtterSec verified-programs registry.
#
# PREREQUISITES
#   - Docker running (used for the deterministic build environment).
#   - solana-verify:   cargo install solana-verify
#   - The repository pushed to a PUBLIC Git URL (verification clones it).
#   - The DEPLOYED program bytecode must equal the reproducible build hash.
#     If the live program was deployed from a non-reproducible local build,
#     redeploy the reproducible artifact first (see step 2).
#
# USAGE
#   ./scripts/verify.sh <cluster> <program_id> <repo_url>
#   e.g. ./scripts/verify.sh devnet 6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ \
#          https://github.com/Worqen-Labs/Worqen-Solana
set -euo pipefail

CLUSTER="${1:-devnet}"
PROGRAM_ID="${2:-6FtagT9Xm9b6eBHgDmxggam2KuiQbPYywUXnrs7B2gEJ}"
REPO_URL="${3:-https://github.com/Worqen-Labs/Worqen-Solana}"
LIBRARY_NAME="worqen_escrow"

command -v solana-verify >/dev/null 2>&1 || { echo "solana-verify not installed: cargo install solana-verify"; exit 1; }
docker info >/dev/null 2>&1 || { echo "Docker is not running."; exit 1; }

echo "==> 1. Reproducible build (Docker)"
solana-verify build --library-name "$LIBRARY_NAME"

echo "==> 2. Reproducible build hash (must match the on-chain program):"
solana-verify get-executable-hash "target/deploy/${LIBRARY_NAME}.so"
echo "    On-chain program hash:"
solana-verify get-program-hash -u "$CLUSTER" "$PROGRAM_ID" || true
echo "    If these differ, redeploy the reproducible artifact:"
echo "      anchor deploy --provider.cluster $CLUSTER (or solana program deploy target/deploy/${LIBRARY_NAME}.so)"

echo "==> 3. Verify against the public repo and write the on-chain record"
solana-verify verify-from-repo \
  --remote \
  -u "$CLUSTER" \
  --program-id "$PROGRAM_ID" \
  --library-name "$LIBRARY_NAME" \
  "$REPO_URL"

echo "==> Done. Explorers will show 'Program is verified' and the security.txt once indexed."
