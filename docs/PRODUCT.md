# DBC Product Specification (v1.0)

This document defines the **installable app** product. Protocol rules live in
[DBC_Whitepaper_Public.md](DBC_Whitepaper_Public.md). Advanced operators may use
the CLI documented in [../README.md](../README.md).

## Principles (from the white paper)

- **Fair launch** — no ICO, no sale, no founder allocation, no privileged role.
- **Equal users** — user #1 and user #10,000 run the **same installer** and **same UI**.
- **CPU sovereignty** — mining on commodity hardware via BritishWork (2048 MiB).
- **It belongs to whoever runs a node** — the network is the set of people who choose to run it.

## Target user

Someone who wants to mine and hold DBC without using a command prompt. They are
not expected to know libp2p, multiaddrs, genesis import, or RocksDB.

## Single user journey (Windows app)

| Step | User action | App behaviour (hidden) |
|------|-------------|------------------------|
| 1 | Run installer | Copy binaries, genesis, peers.enc, guides |
| 2 | Open **DBC Launcher** | Ensure `./data`, import genesis if empty |
| 3 | **Create Wallet** or **Restore Wallet** | BIP-39 wallet; show 24 words once; save `dbc1` address only |
| 4 | **Start** | Run node on `:8333`, enable mining, dial encrypted peer pool, listen |
| 5 | Progress panel | Online/offline, peers, chain height, mining status |
| 6 | **Check balance** | Query UTXO set (brief pause if DB locked) |
| 7 | **Send** | Briefly stops node if running; prompt for 24 words; never store mnemonic |
| 8 | **Stop** | Stop node and mining |

No separate “host mode”, “founder mode”, or manual bootstrap flags in the UI.

## Out of scope for the app (v1)

- Command-line workflows for normal users
- Special treatment for the first person who installs
- Promising perfect anonymity (see white paper §Security model)

## Success criteria

See [ACCEPTANCE.md](ACCEPTANCE.md).
