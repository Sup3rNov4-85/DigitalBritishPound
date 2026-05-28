# Digital British Coin (DBC)
## Public Whitepaper (v1.0)

**A sovereign, decentralised proof-of-work currency** built from first principles.

> “The Times 27/May/2026 — A nation overtaxed, underserved, and searching for an alternative.”

DBC is not a token and is not affiliated with any existing chain. It carries **no premine**, **no founder allocation**, and **no central authority**. It belongs to whoever runs a node.

---

## Abstract

DBC is a peer-to-peer electronic cash system secured by proof-of-work and validated by full nodes. The design targets **CPU-oriented mining** using a memory-hard algorithm (“BritishWork”) to discourage ASIC/GPU centralisation. Total supply is **fixed at 42,000,000 DBC** and emitted via an **8‑year halving schedule**. The system uses modern cryptographic primitives (Schnorr signatures, Bech32m addresses) and an uncle mechanism to reduce selfish mining incentives.

---

## Design goals

- **Fair launch**: no ICO, no sale, no founder allocation.
- **CPU sovereignty**: mining that remains practical on commodity hardware.
- **Long-lived issuance**: a schedule intended to sustain participation across decades.
- **Simple validation**: deterministic rules that any node can enforce independently.
- **Ungovernable by design**: no privileged keys or admin controls in the protocol.

---

## Mining and consensus

### BritishWork (memory-hard PoW)

DBC uses a RandomX-inspired, memory-hard proof-of-work algorithm called **BritishWork**. The intent is to increase the cost of specialised hardware advantage by requiring significant memory and CPU characteristics (branching, cache behaviour) that favour commodity CPUs.

**Mainnet parameters (release builds):**

- **BritishWork memory**: **2048 MiB** (fixed — consensus)
- **Target block interval**: **15 minutes**
- **Difficulty adjustment**: every **1,008** blocks using a weighted moving average window of **144** blocks
- **Max block size**: **2 MB**
- **Uncles**: max **2** per block, lookback **7** blocks, uncle reward **75%** of subsidy
- **Coinbase maturity**: **100** blocks

### Supply schedule

- **Total supply**: 42,000,000 DBC
- **Initial reward**: 50 DBC
- **Halving interval**: 420,000 blocks (~8 years @ 15 minutes)

This schedule is designed to extend issuance across a longer horizon than Bitcoin, keeping incentives for independent miners and full nodes over many decades.

---

## Transactions, scripts, and wallet format

DBC uses a UTXO model (similar to Bitcoin):

- Coins exist as **unspent transaction outputs (UTXOs)**.
- Spending requires creating a new transaction that consumes UTXOs and creates new outputs.
- Nodes enforce standard rules: signatures must be valid, inputs must exist and be unspent, coinbase must respect maturity.

### Addresses

- Human-readable **Bech32m** addresses with `dbc1` prefix.
- Wallets derive keys from a **24‑word mnemonic** (BIP‑39) and a deterministic derivation path (BIP‑32).

**Important:** the mnemonic is the wallet. Anyone who has it can spend the coins.

---

## Networking (peer-to-peer)

DBC nodes communicate over an encrypted P2P network. Nodes relay blocks and transactions and synchronize the chain from peers. Peer discovery is decentralised (DHT) and can be aided by community-operated bootstrap peers.

**This public whitepaper intentionally does not list any IP addresses, peer IDs, or bootstrap endpoints.**

---

## Security model & limitations (read this)

This section is here to avoid accidental security mistakes during public sharing.

### What DBC protects

- **No one can spend your coins** without your private keys/mnemonic.
- **Nodes can verify** the full chain deterministically from the same genesis and rules.
- **Proof-of-work** makes rewriting history expensive and scales with honest participation.

### What DBC does NOT protect (by itself)

- **Perfect anonymity**: blockchains are public ledgers. Addresses are pseudonymous, not invisible.
- **Key theft on an infected PC**: malware can steal mnemonics, clipboard contents, or replace destination addresses.
- **Early-network centralisation**: in the first days, a small number of online nodes/miners can dominate simply because the network is small.
- **Social/operational mistakes**: doxxing yourself via accounts, payment trails, reused usernames, or posting network details tied to identity.

### Wallet safety basics (user guidance)

- Write down the 24 words **offline** (paper/metal). Do not screenshot.
- Never paste your mnemonic into websites or chat messages.
- Prefer a dedicated machine for mining/wallet operations if possible.
- Double-check receive/send addresses (clipboard hijackers exist).

### Software maturity

This reference implementation is an early full node prototype. It is not a formal security audit report. Use the software at your own risk and treat it like early Bitcoin-era software: simple, transparent, and best used by careful operators.

---

## Closing statement

DBC is a practical attempt to restore a “first-year Bitcoin” window of participation — open, CPU‑mineable, and owned by whoever runs a node. The protocol is defined by public rules, and the network is defined by the community that chooses to run it.

