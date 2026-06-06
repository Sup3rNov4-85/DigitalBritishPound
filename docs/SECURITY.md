# Security notes (reference node v1.0)

This is not a formal audit. Use early-stage software carefully.

## Wallet

- **Mnemonic = full control.** Never stored on disk by the launcher.
- Send flow accepts mnemonic via a **temporary file** deleted immediately after use.
- Prefer writing recovery words on paper, not screenshots.

## Network

- libp2p with Noise encryption; GossipSub for blocks/txs.
- Encrypted `peers.enc` uses ChaCha20-Poly1305 keyed from genesis hash.
- Bootstrap is DNS + port; peer IDs learned via identify after connect.

## Chain

- Genesis hash is hard-coded in consensus — wrong genesis rejected.
- BritishWork memory fixed at 2048 MiB in release builds.

## Installer

- User-level install (`%LOCALAPPDATA%`) — no admin by default.
- Verify downloads with `SHA256SUMS.txt`.
- Code signing recommended before wide distribution (SmartScreen).

## Reporting

Report issues via the project repository. Do not post mnemonics or private keys.
