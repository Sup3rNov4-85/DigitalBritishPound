# Changelog

## 1.0.1 — 2026-06-10

### Network
- **All miners grind together** when peers are online (removed designated-miner / sync-only mode)
- Bootstrap shortcut removed — block 1 inherits genesis difficulty (`0x1f00ffff`); retarget at 1,008 blocks unchanged
- Encrypted peer pool grows from successful dials; community nodes dial oldest-first, DuckDNS last
- Mining on `spawn_blocking` so P2P accept loop is not blocked
- DuckDNS bootstrap stored DNS-only (stale `/p2p/` ids stripped on startup)
- `mine_ctl.txt` no longer overwritten by node when UI sets mining on

### UI
- Start/Stop in top bar; Send works while online (brief stop/restart)
- Status from `status.json`: `peer_pool_size`, `mining_mode`

### Docs
- GitHub Releases as primary download; README/LAUNCH/FAQ updated
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
