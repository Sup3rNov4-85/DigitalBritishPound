#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

use eframe::egui;

use dbc_node::api::status::read_status;

/// App version (matches Cargo.toml).
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Prevent a visible `cmd.exe` window when spawning `dbc-node.exe` on Windows.
fn apply_no_console(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
}

/// tracing-subscriber colour codes look like garbage in the UI log panel.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            while let Some(&next) = chars.peek() {
                chars.next();
                if next == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}


/// Listen port (matches DuckDNS entry in encrypted peers.enc).
const NODE_LISTEN: &str = "/ip4/0.0.0.0/tcp/8333";

#[derive(Default, Clone)]
struct NodeStatus {
    peer_count: u32,
    /// User-safe network line (no IPs or peer IDs).
    network_line: String,
    /// User-safe mining line.
    mining_line: String,
}

impl NodeStatus {
    fn reset_running(&mut self) {
        self.network_line = "Starting node…".to_string();
        self.mining_line = "Off".to_string();
    }

    fn reset_stopped(&mut self) {
        self.peer_count = 0;
        self.network_line = "Offline".to_string();
        self.mining_line = "Off".to_string();
    }

    fn ingest_log_line(&mut self, line: &str) -> Option<u64> {
        // Never surface IPs, peer IDs, or reachability warnings to the user.
        if line.contains("share this reachability-safe address")
            || line.contains("local peer id:")
            || line.contains("kad routing updated:")
            || line.contains("mDNS disabled")
        {
            return None;
        }

        if line.contains("listen-only") || line.contains("switching to listen-only") {
            self.network_line =
                "Online — listening for peers (same as every other node)".to_string();
            return None;
        }
        if line.contains("retrying peer search while hosting") || line.contains("retrying peer search while") {
            if self.peer_count == 0 {
                self.network_line = "Online — listening, occasionally searching for peers".to_string();
            }
            return None;
        }
        if line.contains("searching encrypted peer list") {
            if self.peer_count == 0 {
                self.network_line = "Searching for network peers…".to_string();
            }
            return None;
        }
        if line.contains("connected to") {
            self.peer_count = self.peer_count.saturating_add(1);
            self.network_line = if self.peer_count == 1 {
                "Connected to the network".to_string()
            } else {
                format!("Connected — {} peers", self.peer_count)
            };
            return None;
        }
        if line.contains("merged") && line.contains("peer") {
            self.network_line = "Connected — peer list updated".to_string();
            return None;
        }
        if line.contains("P2P listening on") || line.contains("encrypted peer list:") {
            if self.peer_count == 0 {
                self.network_line = "Online — searching for peers…".to_string();
            }
            return None;
        }
        if line.contains("listening on /ip4/") {
            if self.peer_count == 0 {
                self.network_line = "Online — searching for peers…".to_string();
            }
            return None;
        }
        if let Some(h) = parse_height_after(line, "chain tip height=") {
            return Some(h);
        }
        if let Some(h) = parse_height_after(line, "accepted block height=") {
            self.network_line = format!("Synced — block height {h}");
            return Some(h);
        }
        if let Some(h) = parse_height_after(line, "synced block height=") {
            self.network_line = format!("Synced — block height {h}");
            return Some(h);
        }
        if let Some(h) = parse_height_after(line, "mined block height=") {
            self.mining_line = format!("Found block {h}!");
            return Some(h);
        }
        if let Some(h) = parse_height_after(line, "mining block height=") {
            self.mining_line =
                format!("Working on block {h} (CPU mining — solo can take hours)…");
            return None;
        }
        if line.contains("first run — added this node") {
            self.network_line = "Registered on peer list".to_string();
        }
        None
    }

    fn set_mining_off(&mut self) {
        self.mining_line = "Off".to_string();
    }

    fn set_mining_ready(&mut self) {
        self.mining_line = "On — waiting to start…".to_string();
    }
}

fn normalize_mnemonic(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_height_after(line: &str, prefix: &str) -> Option<u64> {
    let rest = line.split(prefix).nth(1)?;
    rest.split_whitespace()
        .next()?
        .trim_end_matches(|c: char| !c.is_ascii_digit())
        .parse()
        .ok()
}

pub struct DbcUiApp {
    payout_address: String,
    wallet_mnemonic: Option<String>,
    show_wallet_backup: bool,
    restore_mnemonic_input: String,
    wallet_message: Option<String>,
    watch_include_immature: bool,

    data_dir: PathBuf,
    wallet_file: PathBuf,
    mine_ctl_path: PathBuf,

    node_spawned_with_payout: bool,

    child: Option<Child>,

    mining_enabled: bool,
    chain_height: Option<u64>,
    error: Option<String>,
    balance_message: Option<String>,
    history_message: Option<String>,
    send_to: String,
    send_amount_dbc: String,
    send_fee_dbc: String,
    send_mnemonic_input: String,
    send_message: Option<String>,
    status: NodeStatus,

    log_rx: Option<mpsc::Receiver<String>>,
    /// When the node subprocess was started (for UI fallbacks).
    node_started_at: Option<std::time::Instant>,
}

impl DbcUiApp {
    fn node_exe_path() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let dir = exe.parent()?;
        let node = dir.join("dbc-node.exe");
        if node.exists() {
            Some(node)
        } else {
            None
        }
    }

    fn save_wallet_address(&self) -> Result<(), String> {
        let addr = self.payout_address.trim();
        if addr.is_empty() {
            return Ok(());
        }
        std::fs::write(&self.wallet_file, addr).map_err(|e| e.to_string())
    }

    fn create_wallet(&mut self) -> Result<(), String> {
        if self.child.is_some() {
            return Err("Stop the node before creating a wallet.".to_string());
        }

        let node = Self::node_exe_path()
            .ok_or_else(|| "Could not find dbc-node.exe next to the UI executable.".to_string())?;

        let mut cmd = Command::new(node);
        apply_no_console(&mut cmd);
        let out = cmd
            .arg("wallet-new")
            .output()
            .map_err(|e| e.to_string())?;

        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        if !out.status.success() {
            return Err(text.trim().to_string());
        }

        let mut mnemonic = None;
        let mut address = None;
        for line in text.lines() {
            if let Some(m) = line.strip_prefix("mnemonic: ") {
                mnemonic = Some(m.trim().to_string());
            }
            if let Some(a) = line.strip_prefix("address: ") {
                address = Some(a.trim().to_string());
            }
        }

        let mnemonic = mnemonic.ok_or_else(|| "wallet-new did not return a mnemonic.".to_string())?;
        let address = address.ok_or_else(|| "wallet-new did not return an address.".to_string())?;

        self.wallet_mnemonic = Some(mnemonic);
        self.payout_address = address;
        self.show_wallet_backup = true;
        self.wallet_message = None;
        self.restore_mnemonic_input.clear();
        self.save_wallet_address()?;
        Ok(())
    }

    fn restore_wallet(&mut self) -> Result<(), String> {
        if self.child.is_some() {
            return Err("Stop the node before restoring a wallet.".to_string());
        }

        let normalized = normalize_mnemonic(&self.restore_mnemonic_input);
        if normalized.is_empty() {
            return Err("Enter your 24-word recovery phrase.".to_string());
        }

        let word_count = normalized.split_whitespace().count();
        if word_count != 24 {
            return Err(format!(
                "Recovery phrase must be exactly 24 words (you entered {word_count})."
            ));
        }

        let node = Self::node_exe_path()
            .ok_or_else(|| "Could not find dbc-node.exe next to the UI executable.".to_string())?;

        let mut cmd = Command::new(node);
        apply_no_console(&mut cmd);
        let out = cmd
            .arg("wallet-addr")
            .arg(&normalized)
            .output()
            .map_err(|e| e.to_string())?;

        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        if !out.status.success() {
            return Err(text.trim().to_string());
        }

        let address = text.trim().lines().last().unwrap_or("").trim();
        if address.is_empty() || !address.starts_with("dbc1") {
            return Err("Could not derive an address from that recovery phrase.".to_string());
        }

        self.payout_address = address.to_string();
        self.wallet_mnemonic = None;
        self.show_wallet_backup = false;
        self.restore_mnemonic_input.clear();
        self.save_wallet_address()?;
        self.wallet_message = Some(
            "Wallet restored. Your payout address is loaded — you can Start to mine.".to_string(),
        );
        Ok(())
    }

    fn refresh_chain_height(&mut self) {
        if self.node_running() {
            return;
        }
        if let Some(h) = self.query_chain_height() {
            self.chain_height = Some(h);
        }
    }

    fn query_chain_height(&self) -> Option<u64> {
        let node = Self::node_exe_path()?;
        let mut cmd = Command::new(&node);
        apply_no_console(&mut cmd);
        cmd.args([
            "--data-dir",
            self.data_dir.to_string_lossy().as_ref(),
            "info",
        ]);
        let out = cmd.output().ok()?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("tip height=") {
                let h = rest.split_whitespace().next()?.parse().ok()?;
                return Some(h);
            }
        }
        None
    }

    fn ensure_genesis(&self) -> Result<(), String> {
        let node = Self::node_exe_path()
            .ok_or_else(|| "Could not find dbc-node.exe next to the UI executable.".to_string())?;
        let genesis = node
            .parent()
            .ok_or_else(|| "Invalid install path.".to_string())?
            .join("genesis.json");
        if !genesis.exists() {
            return Ok(());
        }

        let mut cmd = Command::new(&node);
        apply_no_console(&mut cmd);
        cmd.args([
            "--data-dir",
            self.data_dir.to_string_lossy().as_ref(),
            "import-genesis",
            "--genesis",
            genesis.to_string_lossy().as_ref(),
        ]);

        let out = cmd.output().map_err(|e| e.to_string())?;
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );

        if text.contains("imported genesis") {
            return Ok(());
        }
        if text.contains("import skipped") || text.contains("already has blocks") {
            return Ok(());
        }
        if !out.status.success() {
            return Err(text.trim().to_string());
        }
        Ok(())
    }

    fn start_online(&mut self) -> Result<(), String> {
        if self.payout_address.trim().is_empty() {
            return Err("Create a wallet first (or paste your dbc1 payout address).".to_string());
        }
        if self.node_running() {
            return Ok(());
        }
        self.mining_enabled = true;
        self.spawn_node()?;
        self.status.set_mining_ready();
        Ok(())
    }

    fn stop_online(&mut self) {
        self.mining_enabled = false;
        let _ = std::fs::write(&self.mine_ctl_path, "0");
        self.stop_node();
    }

    fn spawn_node(&mut self) -> Result<(), String> {
        if self.child.is_some() {
            return Ok(());
        }

        let node = Self::node_exe_path()
            .ok_or_else(|| "Could not find dbc-node.exe next to the UI executable.".to_string())?;

        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;
        self.save_wallet_address()?;
        self.ensure_genesis()?;
        self.refresh_chain_height();

        std::fs::write(&self.mine_ctl_path, if self.mining_enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())?;

        let data_dir = self.data_dir.to_string_lossy().to_string();
        let mine_ctl_file = self.mine_ctl_path.to_string_lossy().to_string();
        let payout = self.payout_address.trim();

        let bundled_peers = node
            .parent()
            .ok_or_else(|| "Invalid install path.".to_string())?
            .join("peers.enc");

        let mut cmd = Command::new(&node);
        apply_no_console(&mut cmd);
        cmd.arg("--data-dir")
            .arg(data_dir)
            .arg("run")
            .arg("--listen")
            .arg(NODE_LISTEN)
            .arg("--mine-ctl-file")
            .arg(mine_ctl_file)
            .arg("--bundled-peers")
            .arg(bundled_peers.to_string_lossy().as_ref());

        if !payout.is_empty() {
            cmd.arg("--address").arg(payout);
            self.node_spawned_with_payout = true;
        } else {
            self.node_spawned_with_payout = false;
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd.env("RUST_LOG", "dbc_node=info,libp2p=warn");

        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "Missing stdout".to_string())?;
        let stderr = child.stderr.take().ok_or_else(|| "Missing stderr".to_string())?;

        let (tx, rx) = mpsc::channel::<String>();

        {
            let tx = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    let _ = tx.send(line);
                }
            });
        }

        {
            let tx = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let _ = tx.send(line);
                }
            });
        }

        let chain_height = self.chain_height;
        self.status = NodeStatus::default();
        self.status.reset_running();
        if let Some(h) = chain_height {
            self.status.network_line = format!("Running — block height {h}");
        }
        self.log_rx = Some(rx);
        self.child = Some(child);
        self.node_started_at = Some(std::time::Instant::now());
        Ok(())
    }

    fn stop_node(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.log_rx = None;
        self.node_started_at = None;
        self.status.reset_stopped();
    }

    fn migrate_legacy_data(exe_dir: &PathBuf, data_dir: &PathBuf, wallet_file: &PathBuf) {
        let legacy = exe_dir.join("dbc-ui-data");
        if !legacy.is_dir() {
            return;
        }
        let legacy_wallet = legacy.join("wallet_address.txt");
        if legacy_wallet.exists() && !wallet_file.exists() {
            let _ = std::fs::copy(&legacy_wallet, wallet_file);
        }
    }

    fn refresh_from_status_file(&mut self) {
        if let Some(snap) = read_status(&self.data_dir) {
            if let Some(h) = snap.tip_height {
                self.chain_height = Some(h);
            }
            self.status.peer_count = snap.peer_count;
            if snap.mining_enabled && self.status.mining_line == "Off" {
                self.status.set_mining_ready();
            }
        }
    }

    fn run_balance(&self) -> Result<String, String> {
        let node = Self::node_exe_path().ok_or_else(|| "Missing dbc-node.exe next to UI.".to_string())?;

        let mut cmd = Command::new(node);
        apply_no_console(&mut cmd);
        cmd.args([
            "--data-dir",
            self.data_dir.to_string_lossy().as_ref(),
            "balance",
            "--address",
            self.payout_address.trim(),
        ]);
        if self.watch_include_immature {
            cmd.arg("--include-immature");
        }

        let out = cmd.output().map_err(|e| e.to_string())?;
        let mut s = String::new();
        s.push_str(&String::from_utf8_lossy(&out.stdout));
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s.trim().to_string())
    }

    fn run_history(&self) -> Result<String, String> {
        let node = Self::node_exe_path().ok_or_else(|| "Missing dbc-node.exe next to UI.".to_string())?;

        let mut cmd = Command::new(node);
        apply_no_console(&mut cmd);
        cmd.args([
            "--data-dir",
            self.data_dir.to_string_lossy().as_ref(),
            "history",
            "--address",
            self.payout_address.trim(),
            "--limit",
            "10",
        ]);

        let out = cmd.output().map_err(|e| e.to_string())?;
        let mut s = String::new();
        s.push_str(&String::from_utf8_lossy(&out.stdout));
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        Ok(s.trim().to_string())
    }

    fn run_send(&self) -> Result<String, String> {
        if self.payout_address.trim().is_empty() {
            return Err("Set your wallet address first.".to_string());
        }
        let to = self.send_to.trim();
        if !to.starts_with("dbc1") {
            return Err("Recipient must be a dbc1… address.".to_string());
        }
        let amount: u64 = self
            .send_amount_dbc
            .trim()
            .parse()
            .map_err(|_| "Enter a whole-number DBC amount.".to_string())?;
        let fee: u64 = if self.send_fee_dbc.trim().is_empty() {
            0
        } else {
            self.send_fee_dbc
                .trim()
                .parse()
                .map_err(|_| "Fee must be a whole number.".to_string())?
        };
        let normalized = normalize_mnemonic(&self.send_mnemonic_input);
        if normalized.split_whitespace().count() != 24 {
            return Err("Enter your 24-word recovery phrase to authorise sending.".to_string());
        }

        let node = Self::node_exe_path().ok_or_else(|| "Missing dbc-node.exe next to UI.".to_string())?;
        let temp = self.data_dir.join(format!("send_{}.tmp", std::process::id()));
        std::fs::write(&temp, &normalized).map_err(|e| e.to_string())?;

        let mut cmd = Command::new(node);
        apply_no_console(&mut cmd);
        cmd.args([
            "--data-dir",
            self.data_dir.to_string_lossy().as_ref(),
            "send",
            "--from-mnemonic-file",
            temp.to_string_lossy().as_ref(),
            "--to",
            to,
            "--amount-dbc",
            &amount.to_string(),
            "--fee-dbc",
            &fee.to_string(),
        ]);

        let out = cmd.output().map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&temp);
        let mut s = String::new();
        s.push_str(&String::from_utf8_lossy(&out.stdout));
        s.push_str(&String::from_utf8_lossy(&out.stderr));
        if !out.status.success() {
            return Err(s.trim().to_string());
        }
        Ok(s.trim().to_string())
    }

    fn node_running(&self) -> bool {
        self.child.is_some()
    }
}

impl Default for DbcUiApp {
    fn default() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let data_dir = exe_dir.join("data");
        let mine_ctl_path = data_dir.join("mine_ctl.txt");
        let wallet_file = data_dir.join("wallet_address.txt");
        Self::migrate_legacy_data(&exe_dir, &data_dir, &wallet_file);

        let payout_address = std::fs::read_to_string(&wallet_file)
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let mut app = Self {
            payout_address,
            wallet_mnemonic: None,
            show_wallet_backup: false,
            restore_mnemonic_input: String::new(),
            wallet_message: None,
            watch_include_immature: false,
            data_dir,
            wallet_file,
            mine_ctl_path,
            node_spawned_with_payout: false,
            child: None,
            mining_enabled: false,
            chain_height: None,
            error: None,
            balance_message: None,
            history_message: None,
            send_to: String::new(),
            send_amount_dbc: String::new(),
            send_fee_dbc: String::new(),
            send_mnemonic_input: String::new(),
            send_message: None,
            status: {
                let mut s = NodeStatus::default();
                s.reset_stopped();
                s
            },
            log_rx: None,
            node_started_at: None,
        };
        let _ = app.ensure_genesis();
        app.refresh_chain_height();
        app
    }
}

impl eframe::App for DbcUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.log_rx {
            while let Ok(line) = rx.try_recv() {
                let clean = strip_ansi(&line);
                if let Some(h) = self.status.ingest_log_line(&clean) {
                    self.chain_height = Some(h);
                }
            }
        }

        if self.node_running() {
            self.refresh_from_status_file();
            if let Some(started) = self.node_started_at {
                if started.elapsed() > std::time::Duration::from_secs(50)
                    && self.status.peer_count == 0
                    && !self.status.network_line.contains("Connected")
                {
                    self.status.network_line =
                        "Online — listening for peers (same as every other node)".to_string();
                }
            }
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.heading(format!("Digital British Coin v{APP_VERSION}"));
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.node_running(), egui::Button::new("Start"))
                    .clicked()
                {
                    if let Err(e) = self.start_online() {
                        self.error = Some(e);
                    }
                }
                if ui
                    .add_enabled(self.node_running(), egui::Button::new("Stop"))
                    .clicked()
                {
                    self.stop_online();
                }
                if self.node_running() {
                    ui.colored_label(egui::Color32::from_rgb(0, 160, 0), "● Online");
                } else {
                    ui.colored_label(egui::Color32::GRAY, "○ Offline");
                }
            });
            ui.label("Start = node + mining. Stop = offline.");
            ui.separator();
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::RED, err);
                ui.separator();
            }
            self.error = None;

            ui.heading("Progress");
            egui::Frame::group(ui.style()).show(ui, |ui| {
                if self.node_running() {
                    ui.colored_label(egui::Color32::from_rgb(0, 160, 0), "● Online");
                } else {
                    ui.colored_label(egui::Color32::GRAY, "○ Offline");
                }
                ui.label(format!("Network: {}", self.status.network_line));
                if self.status.peer_count > 0 {
                    ui.label(format!("Peers: {}", self.status.peer_count));
                }
                if let Some(h) = self.chain_height {
                    ui.label(format!("Chain height: {h}"));
                } else {
                    ui.label("Chain height: —");
                }
                ui.label(format!("Mining: {}", self.status.mining_line));
            });

            if let Some(msg) = &self.balance_message {
                ui.separator();
                ui.label("Balance:");
                ui.monospace(msg);
            }

            ui.separator();
            ui.heading("Wallet");

            egui::Frame::group(ui.style()).show(ui, |ui| {
                ui.label("About your recovery phrase (24 words):");
                ui.label("• Write it on paper when you create a wallet — this app does not save it.");
                ui.label("• You need it to send coins or restore your wallet on a new PC.");
                ui.label("• You do not need it to mine, receive coins, or check your balance.");
                ui.label(
                    "• If you lose it, coins sent to your address cannot be spent or recovered.",
                );
            });

            if let Some(msg) = &self.wallet_message {
                ui.colored_label(egui::Color32::from_rgb(0, 140, 0), msg);
            }

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.node_running(), egui::Button::new("Create Wallet"))
                    .clicked()
                {
                    match self.create_wallet() {
                        Ok(()) => {}
                        Err(e) => self.error = Some(e),
                    }
                }
                ui.label("Generates a new dbc1 address + one-time 24-word backup");
            });

            if self.show_wallet_backup {
                ui.colored_label(
                    egui::Color32::from_rgb(200, 80, 0),
                    "IMPORTANT — save these 24 words on paper now. They are shown once and not stored on disk.",
                );
                if let Some(m) = &self.wallet_mnemonic {
                    egui::ScrollArea::vertical()
                        .max_height(60.0)
                        .show(ui, |ui| {
                            ui.monospace(m);
                        });
                }
                ui.label("Keep them private. You will need them to send DBC or set up on another computer.");
            }

            ui.add_space(4.0);
            ui.label("Restore wallet from recovery phrase:");
            ui.add(
                egui::TextEdit::multiline(&mut self.restore_mnemonic_input)
                    .hint_text("Paste or type all 24 words, separated by spaces…")
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            ui.horizontal(|ui| {
                let can_restore = !self.node_running()
                    && !normalize_mnemonic(&self.restore_mnemonic_input).is_empty();
                if ui
                    .add_enabled(can_restore, egui::Button::new("Restore Wallet"))
                    .clicked()
                {
                    match self.restore_wallet() {
                        Ok(()) => {}
                        Err(e) => self.error = Some(e),
                    }
                }
                ui.label("Loads your dbc1 payout address (phrase is not saved)");
            });

            ui.horizontal(|ui| {
                ui.label("Your address:");
                ui.add(
                    egui::TextEdit::singleline(&mut self.payout_address)
                        .desired_width(f32::INFINITY),
                );
                if ui.button("Copy").clicked() {
                    ctx.copy_text(self.payout_address.clone());
                }
            });

            ui.checkbox(&mut self.watch_include_immature, "Include immature coinbase in balance");

            ui.separator();
            ui.heading("Send");

            ui.horizontal(|ui| {
                ui.label("To (dbc1…):");
                ui.text_edit_singleline(&mut self.send_to);
            });
            ui.horizontal(|ui| {
                ui.label("Amount (DBC):");
                ui.text_edit_singleline(&mut self.send_amount_dbc);
                ui.label("Fee:");
                ui.text_edit_singleline(&mut self.send_fee_dbc);
            });
            ui.label("Recovery phrase (24 words — required to send, not saved):");
            ui.add(
                egui::TextEdit::multiline(&mut self.send_mnemonic_input)
                    .desired_rows(2)
                    .desired_width(f32::INFINITY),
            );
            if ui
                .add_enabled(!self.node_running(), egui::Button::new("Send"))
                .clicked()
            {
                let was_running = self.node_running();
                if was_running {
                    self.stop_online();
                }
                match self.run_send() {
                    Ok(msg) => {
                        self.send_message = Some(msg);
                        self.send_mnemonic_input.clear();
                        self.refresh_chain_height();
                    }
                    Err(e) => self.error = Some(e),
                }
                if was_running {
                    if let Err(e) = self.start_online() {
                        self.error = Some(e);
                    }
                }
            }
            if let Some(msg) = &self.send_message {
                ui.monospace(msg);
            }
            ui.label("Stop the node before sending if you see a database lock error.");

            ui.separator();

            ui.horizontal(|ui| {
                if ui.button("Check balance").clicked() {
                    let was_running = self.node_running();
                    if was_running {
                        self.stop_online();
                    }
                    self.refresh_chain_height();
                    match self.run_balance() {
                        Ok(txt) => self.balance_message = Some(txt),
                        Err(e) => self.error = Some(e),
                    }
                    if was_running {
                        if let Err(e) = self.start_online() {
                            self.error = Some(e);
                        }
                    }
                }
                if ui.button("Recent activity").clicked() {
                    let was_running = self.node_running();
                    if was_running {
                        self.stop_online();
                    }
                    match self.run_history() {
                        Ok(txt) => self.history_message = Some(txt),
                        Err(e) => self.error = Some(e),
                    }
                    if was_running {
                        if let Err(e) = self.start_online() {
                            self.error = Some(e);
                        }
                    }
                }
            });
            if let Some(msg) = &self.history_message {
                ui.label("Recent activity:");
                ui.monospace(msg);
            }
                });
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([540.0, 680.0])
            .with_min_inner_size([480.0, 420.0]),
        ..Default::default()
    };
    eframe::run_native(
        "DBC Launcher",
        options,
        Box::new(|_cc| Box::<DbcUiApp>::default()),
    )
}
