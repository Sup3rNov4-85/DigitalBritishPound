Digital British Coin (DBC) — Release Package

Genesis hash (MUST match):
87f9442d436c6627f00a4bc025e149d0c2fe30dc5f77eb2c18acd086ba582a7d

Bootstrap seed (first node):
/ip4/176.24.48.191/tcp/8333/p2p/12D3KooWAmFcBBrh2H2SQQ5u2b2LU57kAToYKx18xct5zh3NVy7m

Quick start (Windows PowerShell)
1) Create a wallet (SAVE the 24 words; never share them):
   .\dbc-node.exe wallet-new

2) Start node + mine (replace dbc1... with YOUR address from wallet-new):
   .\dbc-node.exe run --listen /ip4/0.0.0.0/tcp/8334 `
     --bootstrap /ip4/176.24.48.191/tcp/8333/p2p/12D3KooWAmFcBBrh2H2SQQ5u2b2LU57kAToYKx18xct5zh3NVy7m `
     --mine --address dbc1PASTE_YOUR_ADDRESS_HERE

Wallet basics
- Receive: share your dbc1... address.
- Check balance (stop your node first if it's running on the same data dir):
  .\dbc-node.exe --data-dir .\data balance --address dbc1PASTE_YOUR_ADDRESS_HERE

- Send (mnemonic is private; never share it):
  .\dbc-node.exe --data-dir .\data send --from-mnemonic "word1 word2 ... word24" --to dbc1RECIPIENT --amount-dbc 1 --fee-dbc 0

Notes
- Do NOT run `init` (it would create a different chain).
- If you can't connect, the seed operator may not have TCP 8333 forwarded correctly.
- Full guide: DBC_Node_README.pdf
