# Changelog

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
