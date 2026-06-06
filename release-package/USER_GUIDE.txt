# Digital British Coin — User Guide

**Install the app. Create a wallet. Press Start.**

This guide is for the Windows **DBC Launcher**. You do not need a command prompt.

## Install

1. Download `dbc-installer.exe` from the official release link.
2. Run the installer (installs to your user folder — no admin required).
3. Open **Digital British Coin (DBC) → DBC Launcher** from the Start Menu.

## Create your wallet

1. Click **Create Wallet**.
2. **Write down the 24 words on paper.** The app does not save them.
3. Your **dbc1…** address appears — this is where mining rewards go.

To use an existing wallet: paste your 24 words under **Restore Wallet** and click
**Restore Wallet**.

## Go online and mine

1. Click **Start** — the node connects to the network and mining begins.
2. **Progress** shows status, peers, chain height, and mining.
3. Click **Stop** when you want to go offline.

**Start = node + mining. Stop = offline.**

Solo CPU mining can take **minutes to hours** per block (BritishWork — one core, fair for all hardware).

## Balance

Click **Check balance**. Mining rewards need **100 blocks** (~25 hours) before
they become spendable.

## Send coins

1. Stop is not required for send if you have spendable balance.
2. Open **Send**, enter recipient `dbc1…`, amount, and your **24 words** (only for
   sending — never share them).
3. Confirm **Send**.

## Recovery phrase — when you need it

| Task | Need 24 words? |
|------|----------------|
| Mine / receive / check balance | No — address only |
| Send coins | Yes |
| New PC | Yes — Restore Wallet |

## Tips

- Allow **dbc-node** through Windows Firewall if prompted (port 8333).
- On a small network, **0 peers** for a while is normal until others join.
- Do **not** run any “init genesis” tool — the app imports the official genesis for you.

## Genesis (verify)

Official genesis hash:

`87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d`

## Help

See [FAQ.md](FAQ.md). Protocol details: [DBC_Whitepaper_Public.pdf](DBC_Whitepaper_Public.pdf).
