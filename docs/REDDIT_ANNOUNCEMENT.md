# Reddit / forum announcement (copy-paste)

**Title:** [Release] Digital British Coin (DBC) — open-source CPU PoW node, fair launch (42M cap, no premine)

**Body:**

I’ve published an early reference full-node for **Digital British Coin (DBC)** — a proof-of-work UTXO chain (not a token), with a public genesis and no ICO/premine.

**Download (Windows zip):** https://drive.google.com/file/d/1P1onJ4yWRWSDd5GAt3Be9ooBDCL7S-Oi/view?usp=drive_link

**Source + docs:** https://github.com/Sup3rNov4-85/DigitalBritishPound

**Genesis hash (verify):**  
`87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d`

**Highlights**
- 42,000,000 fixed supply, ~15 min target, 8-year halving
- CPU-oriented memory-hard PoW (“BritishWork”, 2048 MiB in release builds)
- Schnorr + Bech32m `dbc1` addresses, BIP-39/32 wallet
- Uncle blocks, libp2p P2P

**Quick start**
1. `wallet-new` — save your 24 words offline  
2. `run` with bootstrap from README — **do not run `init`**  
3. Optional: start mining in the UI (set payout `dbc1...`)

**Bootstrap**
```
/dns4/digitalbritishpound.duckdns.org/tcp/8333/p2p/12D3KooWAmFcBBrh2H2SQQ5u2b2LU57kAToYKx18xct5zh3NVy7m
```

Early software — run at your own risk. Verify `SHA256SUMS.txt`. Never share your seed phrase.

Whitepaper screenshots in comments / see repo `docs/DBC_Whitepaper_Public.pdf`.

---

**Subreddit notes**
- `r/cryptomining` / `r/gpumining`: frame as CPU mining experiment  
- `r/CryptoCurrency`: be factual, no price talk, expect moderation  
- Read each sub’s rules before posting
