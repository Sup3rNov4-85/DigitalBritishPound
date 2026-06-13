# Changelog

## 1.0.2 — 2026-06-10

### Network
- Never stop outbound peer search (removed listen-only after 45s); dial every 30s while offline
- UPnP external address registered in peer pool so other miners can find you
- Identify: learn peer listen addresses and dial them; merge into encrypted pool
- Sync every 5s while connected + every 30s peer tick + before each mine attempt
- Abort stale mining when a network block arrives; wake miner for next height immediately
- Launcher enables mDNS so friends on the same LAN discover each other

### UI
- Peers: 0 warning when solo; chain height updates when sync catches up from status.json
- Download links point to official Google Drive installer

## 1.0.1 — 2026-06-10

### Network
- Bootstrap shortcut removed — block 1 inherits genesis difficulty (`0x1f00ffff`); retarget at 1,008 blocks unchanged
- Encrypted peer pool grows from successful dials; community nodes dial oldest-first, DuckDNS last
- Mining on `spawn_blocking` so P2P accept loop is not blocked
- DuckDNS bootstrap stored DNS-only (stale `/p2p/` ids stripped on startup)
- `mine_ctl.txt` no longer overwritten by node when UI sets mining on

### UI
- Start/Stop in top bar; Send works while online (brief stop/restart)
- Status from `status.json`: `peer_pool_size`, `mining_mode`

### Docs
- Official Windows installer download via Google Drive; README/LAUNCH/FAQ updated
- USER_GUIDE, README, REDDIT aligned with DNS-only bootstrap and Launcher flow

## 1.0.0 — 2026-06-05

### Product (Windows app)
- Single flow for all users: Install → Wallet → Start/Stop
- Wallet create, restore, send (with recovery phrase at send time)
- Balance and recent activity from the launcher
- Progress panel: peers, chain height, mining status via status.json
- No founder/host mode in the UI — equal experience per white paper

### Network
- Encrypted peers.enc with DNS bootstrap (no stale hardcoded peer IDs)
- Auto listen-only after failed peer search; periodic retry for all nodes
- Peer list merge over P2P

### Node
- `history`, `serve` (localhost read-only API), `--from-mnemonic-file` for send
- Chain tip written to status.json while running

### Docs
- PRODUCT.md, USER_GUIDE.md, FAQ.md, ACCEPTANCE.md, SECURITY.md, ROADMAP_V2.md
