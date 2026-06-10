# DBC — Frequently Asked Questions

## Do I need a command prompt?

No. Use **DBC Launcher** only. The command-line tools are for developers.

## Is there a founder or special first user?

No. The white paper defines a **fair launch** with no founder allocation. Everyone
uses the same installer and the same **Start** button.

## Why does it say “0 peers” or “listening for peers”?

The network grows as more people install and press Start. On a new network it is
normal to see no peers until others are online. Your node still syncs genesis and
can mine locally.

## Why is my balance not spendable yet?

Mining rewards (coinbase) need **100 blocks** to mature (~25 hours at 15-minute
blocks). Until then they show as confirmed but not spendable.

## I lost my 24 words. Can you recover my coins?

No. The app never stores your recovery phrase. Without it, coins at your address
cannot be spent. You can still mine **to** an address if you only saved the `dbc1`
string, but you cannot send or restore on a new PC.

## Do I need to port-forward my router?

Not required to mine or sync genesis. Other nodes can only connect **to you** if
port **8333** is reachable from the internet. Many home users never need this —
outbound connections to other peers are enough once the network has public seeds.

## Windows Firewall blocked something

Allow **dbc-node** on private networks when prompted. Port **8333** is used for
peer connections.

## How do I verify the download?

Download `dbc-installer.exe` from [GitHub Releases](https://github.com/Sup3rNov4-85/DigitalBritishPound/releases) (or the [Google Drive mirror](https://drive.google.com/file/d/151Oy8REpkWhEjHVG6qPafDDJH3v1-HDn/view?usp=sharing)) and compare file hashes to `SHA256SUMS.txt` in the release package or repo.

## What is the official genesis hash?

`87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d`

## Is this anonymous?

The blockchain is a public ledger. Addresses are pseudonymous, not invisible. See
the white paper §Security model.
