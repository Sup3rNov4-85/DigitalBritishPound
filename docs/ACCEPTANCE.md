# DBC v1.0 Acceptance Criteria

Binary pass/fail gates before calling the product complete.

## App (every user, Windows)

- [ ] Fresh VM: install → Create Wallet → Start → chain height visible within 60s
- [ ] Mining starts after Start (mining line shows work or found block)
- [ ] Stop returns to Offline
- [ ] Restore Wallet loads correct `dbc1` from 24 words
- [ ] Check balance shows confirmed/spendable without user-visible errors
- [ ] Send moves spendable DBC to another `dbc1` (with mnemonic at send time)
- [ ] No user-facing text mentions “founder”, “host mode”, or manual bootstrap
- [ ] Single data directory: `./data` only

## Network (equal users)

- [ ] Two independent installs connect without CLI flags
- [ ] Blocks propagate between connected peers
- [ ] Encrypted `peers.enc` merges over P2P
- [ ] Bootstrap uses DNS:port without stale hardcoded peer IDs
- [ ] After ~45s with no peers, node listens and retries periodically (same for all)

## Release

- [ ] `dbc-installer.exe` bundles verified files (`SHA256SUMS.txt`)
- [ ] Genesis hash matches white paper / `genesis.json`
- [ ] User guide PDF/MD shipped with installer
- [ ] Version shown in launcher title

## Protocol (white paper v1.0)

- [ ] Genesis hash locked: `87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d`
- [ ] BritishWork 2048 MiB, halving, uncles, coinbase maturity 100 blocks
- [ ] No premine in genesis coinbase beyond height 0 rules

## Documented future work (not v1 blockers)

- Full RandomX BritishWork, SPV, full HTTP RPC — see [ROADMAP_V2.md](ROADMAP_V2.md)
