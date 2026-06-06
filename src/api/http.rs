//! Local read-only HTTP API (localhost). Full RPC is v2 — see docs/ROADMAP_V2.md.

use std::net::SocketAddr;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

use crate::{
    api::status::read_status,
    crypto::wallet::{format_units_as_dbc, Address},
    node::chain::Chain,
};

pub async fn serve_local(
    bind: SocketAddr,
    data_dir: std::path::PathBuf,
    chain: Chain,
) -> anyhow::Result<()> {
    let listener = TcpListener::bind(bind).await?;
    tracing::info!("local API listening on http://{bind}/ (GET /status /balance?address=dbc1...)");

    loop {
        let (mut stream, _) = listener.accept().await?;
        let data_dir = data_dir.clone();
        let chain = chain.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_connection(&mut stream, &data_dir, &chain).await {
                tracing::debug!("api connection: {e}");
            }
        });
    }
}

async fn handle_connection(
    stream: &mut TcpStream,
    data_dir: &std::path::Path,
    chain: &Chain,
) -> anyhow::Result<()> {
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    if n == 0 {
        return Ok(());
    }
    let req = String::from_utf8_lossy(&buf[..n]);
    let path = req.lines().next().unwrap_or("");
    let path = path.split_whitespace().nth(1).unwrap_or("/");

    let (status, body) = if path == "/status" || path.starts_with("/status?") {
        let snap = read_status(data_dir).unwrap_or_default();
        (200, serde_json::to_string_pretty(&snap)?)
    } else if let Some(q) = path.strip_prefix("/balance?") {
        let addr = parse_query(q, "address").ok_or_else(|| anyhow::anyhow!("missing address"))?;
        (200, balance_json(chain, &addr)?)
    } else if path == "/" {
        (
            200,
            r#"{"endpoints":["GET /status","GET /balance?address=dbc1..."]}"#.to_string(),
        )
    } else {
        (404, r#"{"error":"not found"}"#.to_string())
    };

    let response = format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    Ok(())
}

fn parse_query(q: &str, key: &str) -> Option<String> {
    for part in q.split('&') {
        if let Some((k, v)) = part.split_once('=') {
            if k == key {
                return Some(v.to_string());
            }
        }
    }
    None
}

fn balance_json(chain: &Chain, address: &str) -> anyhow::Result<String> {
    let addr = Address::from_bech32m(address)?;
    let current_height = chain.tip()?.map(|t| t.height).unwrap_or(0);
    let mut total = 0u64;
    let mut spendable = 0u64;
    chain.utxos().for_each(|_op, utxo| {
        if utxo.output.script_pubkey.as_bytes().len() == 21
            && utxo.output.script_pubkey.as_bytes()[0] == 0x14
            && &utxo.output.script_pubkey.as_bytes()[1..21] == addr.as_bytes()
        {
            total = total.saturating_add(utxo.value());
            if utxo.is_mature(current_height) {
                spendable = spendable.saturating_add(utxo.value());
            }
        }
        Ok(())
    })?;
    Ok(serde_json::json!({
        "address": addr.to_bech32m()?,
        "confirmed_dbc": format_units_as_dbc(total),
        "spendable_dbc": format_units_as_dbc(spendable),
        "tip_height": current_height,
    })
    .to_string())
}
