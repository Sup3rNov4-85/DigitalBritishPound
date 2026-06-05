# DBC launch information

## Download (Windows package)

**Official release zip:** [Google Drive](https://drive.google.com/file/d/1P1onJ4yWRWSDd5GAt3Be9ooBDCL7S-Oi/view?usp=drive_link)

Contents: `dbc-node.exe`, `genesis.json`, `README.txt`, `DBC_Node_README.pdf`, `SHA256SUMS.txt`

Verify file hashes with `SHA256SUMS.txt` after download.

## Official genesis

- **Genesis hash:** `87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d`
- **Do not run `init`** unless you intend to create a different chain.

## Join the network (bootstrap)

```
/dns4/digitalbritishpound.duckdns.org/tcp/8333/p2p/12D3KooWAmFcBBrh2H2SQQ5u2b2LU57kAToYKx18xct5zh3NVy7m
```

## Quick start (Windows)

```powershell
.\dbc-node.exe wallet-new
.\dbc-node.exe run --listen /ip4/0.0.0.0/tcp/8334 `
  --bootstrap /dns4/digitalbritishpound.duckdns.org/tcp/8333/p2p/12D3KooWAmFcBBrh2H2SQQ5u2b2LU57kAToYKx18xct5zh3NVy7m
```

### Start mining / stop mining (via UI)

Once the node is running, use the Windows UI to:
- set your **payout address** (`dbc1...`)
- click **Start Miner**
- click **Stop Miner** to pause mining

## Whitepaper

See `docs/DBC_Whitepaper_Public.pdf` in this repository.

## Build from source

```bash
cargo build --release
```

BritishWork PoW memory is fixed at **2048 MiB** in release builds (consensus).
