# DBC launch information

## Download (Windows)

**Official release:** [Google Drive — `dbc-installer.exe`](https://drive.google.com/file/d/1fHCBOKxuf4bjEJXtgzF1IjD1JHwI9bpJ/view?usp=sharing)

**From source:** clone [github.com/Sup3rNov4-85/DigitalBritishPound](https://github.com/Sup3rNov4-85/DigitalBritishPound) and run `scripts/build_release.ps1` (requires Rust + NSIS).

Verify file hashes with `SHA256SUMS.txt` after download.

### Install
1. Run `dbc-installer.exe` (user folder — no admin required).
2. Open **Digital British Coin (DBC) → DBC Launcher**.
3. Create wallet → **Start**.

Every user follows the same steps. There is no special “first user” mode.

## Official genesis

- **Genesis hash:** `87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d`
- **Do not run `init`** — the app imports `genesis.json` automatically.

## Network

Nodes discover peers via encrypted `peers.enc` (shipped with the installer), Kademlia DHT, and merged peer lists over P2P.

Bootstrap uses DNS + port only: `/dns4/digitalbritishpound.duckdns.org/tcp/8333`

Community volunteers may run additional public seeds (VPS) and share multiaddrs — optional, not required for v1.

## Build from source

```powershell
.\scripts\build_release.ps1
```

## Documents

| Doc | Audience |
|-----|----------|
| [USER_GUIDE.md](USER_GUIDE.md) | App users |
| [FAQ.md](FAQ.md) | App users |
| [PRODUCT.md](PRODUCT.md) | Product spec |
| [DBC_Whitepaper_Public.pdf](DBC_Whitepaper_Public.pdf) | Protocol |
| [../README.md](../README.md) | Developers / CLI |
