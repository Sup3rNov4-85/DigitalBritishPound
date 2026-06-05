use std::collections::VecDeque;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

use eframe::egui;

const DEFAULT_BOOTSTRAP: &str =
    "/dns4/digitalbritishpound.duckdns.org/tcp/8333/p2p/12D3KooWAmFcBBrh2H2SQQ5u2b2LU57kAToYKx18xct5zh3NVy7m";

const DEFAULT_LISTEN: &str = "/ip4/0.0.0.0/tcp/8334";

#[derive(Default)]
struct UiLog {
    lines: VecDeque<String>,
}

impl UiLog {
    fn push(&mut self, line: String) {
        const MAX: usize = 200;
        if self.lines.len() >= MAX {
            self.lines.pop_front();
        }
        self.lines.push_back(line);
    }
}

pub struct DbcUiApp {
    payout_address: String,
    watch_include_immature: bool,

    data_dir: PathBuf,
    mine_ctl_path: PathBuf,

    seed_node_listening: bool,
    child: Option<Child>,

    mining_enabled: bool,
    error: Option<String>,
    log: UiLog,

    // Receive stdout/stderr lines from the running node.
    log_rx: Option<mpsc::Receiver<String>>,
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

    fn spawn_node(&mut self) -> Result<(), String> {
        if self.child.is_some() {
            return Ok(());
        }

        let node = Self::node_exe_path()
            .ok_or_else(|| "Could not find dbc-node.exe next to the UI executable.".to_string())?;

        if self.payout_address.trim().is_empty() {
            return Err("Set your payout address (dbc1...) first.".to_string());
        }

        std::fs::create_dir_all(&self.data_dir).map_err(|e| e.to_string())?;

        // Tell the node to begin/stop mining without restarting.
        std::fs::write(&self.mine_ctl_path, if self.mining_enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())?;

        // Start P2P node (mining controlled by file).
        let data_dir = self.data_dir.to_string_lossy().to_string();
        let mine_ctl_file = self.mine_ctl_path.to_string_lossy().to_string();
        let payout = self.payout_address.trim().to_string();

        let mut cmd = Command::new(&node);
        cmd.arg("--data-dir")
            .arg(data_dir)
            .arg("run")
            .arg("--listen")
            .arg(DEFAULT_LISTEN)
            .arg("--bootstrap")
            .arg(DEFAULT_BOOTSTRAP)
            .arg("--mine-ctl-file")
            .arg(mine_ctl_file)
            .arg("--address")
            .arg(payout);

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let stdout = child.stdout.take().ok_or_else(|| "Missing stdout".to_string())?;
        let stderr = child.stderr.take().ok_or_else(|| "Missing stderr".to_string())?;

        let (tx, rx) = mpsc::channel::<String>();

        // stdout reader
        {
            let tx = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stdout);
                for line in reader.lines().flatten() {
                    let _ = tx.send(line);
                }
            });
        }

        // stderr reader
        {
            let tx = tx.clone();
            thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().flatten() {
                    let _ = tx.send(line);
                }
            });
        }

        self.log_rx = Some(rx);
        self.child = Some(child);
        Ok(())
    }

    fn stop_node(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
        self.log_rx = None;
    }

    fn set_mining_enabled(&mut self, enabled: bool) -> Result<(), String> {
        self.mining_enabled = enabled;
        std::fs::write(&self.mine_ctl_path, if enabled { "1" } else { "0" })
            .map_err(|e| e.to_string())
    }

    fn run_balance(&self) -> Result<String, String> {
        let node = Self::node_exe_path().ok_or_else(|| "Missing dbc-node.exe next to UI.".to_string())?;

        let mut cmd = Command::new(node);
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
}

impl Default for DbcUiApp {
    fn default() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));

        let data_dir = exe_dir.join("dbc-ui-data");
        let mine_ctl_path = data_dir.join("mine_ctl.txt");

        Self {
            payout_address: String::new(),
            watch_include_immature: false,
            data_dir,
            mine_ctl_path,
            seed_node_listening: false,
            child: None,
            mining_enabled: false,
            error: None,
            log: UiLog::default(),
            log_rx: None,
        }
    }
}

impl eframe::App for DbcUiApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(rx) = &self.log_rx {
            // Drain log queue.
            while let Ok(line) = rx.try_recv() {
                self.log.push(line);
            }
        }

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.heading("DBC — Windows Launcher (seed + mine control)");
            ui.separator();
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(err) = &self.error {
                ui.colored_label(egui::Color32::RED, err);
                ui.separator();
            }
            self.error = None;

            ui.horizontal(|ui| {
                ui.label("Payout address (dbc1...):");
                ui.text_edit_singleline(&mut self.payout_address);
            });

            ui.checkbox(&mut self.watch_include_immature, "Include immature coinbase");

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(!self.child.is_some(), egui::Button::new("Start Node"))
                    .clicked()
                {
                    if let Err(e) = self.spawn_node() {
                        self.error = Some(e);
                    }
                }

                if ui
                    .add_enabled(self.child.is_some(), egui::Button::new("Stop Node"))
                    .clicked()
                {
                    self.stop_node();
                }
            });

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(self.child.is_some(), egui::Button::new("Start Miner"))
                    .clicked()
                {
                    if let Err(e) = self.set_mining_enabled(true) {
                        self.error = Some(e);
                    }
                }
                if ui
                    .add_enabled(self.child.is_some(), egui::Button::new("Stop Miner"))
                    .clicked()
                {
                    if let Err(e) = self.set_mining_enabled(false) {
                        self.error = Some(e);
                    }
                }
            });

            ui.separator();

            if ui.button("Check balance").clicked() {
                // Balance needs exclusive DB access with this prototype, so we stop the node temporarily.
                let was_running = self.child.is_some();
                if was_running {
                    self.stop_node();
                }
                match self.run_balance() {
                    Ok(txt) => {
                        self.error = None;
                        self.log.push(format!("--- balance ---\n{txt}\n--- end ---"));
                    }
                    Err(e) => self.error = Some(e),
                }
                if was_running {
                    // Restart with the same mining toggle.
                    if let Err(e) = self.spawn_node() {
                        self.error = Some(e);
                    } else {
                        // Ensure mining state matches the UI toggle.
                        let _ = self.set_mining_enabled(self.mining_enabled);
                    }
                }
            }

            ui.separator();
            ui.label("Node log (stdout/stderr):");
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                for line in &self.log.lines {
                    ui.monospace(line);
                    ui.add_space(1.0);
                }
            });

            ui.separator();
            ui.label(
                "Tip: mining is controlled by mine_ctl.txt. Start miner writes '1', stop writes '0'.",
            );
        });

        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions::default();
    eframe::run_native(
        "DBC Launcher",
        options,
        Box::new(|_cc| Box::<DbcUiApp>::default()),
    )
}

