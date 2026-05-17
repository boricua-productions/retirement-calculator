use eframe::egui;
use egui::{Color32, FontData, FontDefinitions, FontFamily, RichText};
use std::sync::mpsc;

use crate::config::loader::load_scenario;
use crate::engine::rsu_engine::RsuEngine;
use crate::models::snapshot::SimResults;
use crate::reporter;
use crate::simulation::controller::SimulationController;
use crate::simulation::monte_carlo::{
    self, MarcoPoloInput, MarcoPoloResults, DEFAULT_ITERATIONS,
};
use crate::ui::panels::{
    chart_panel, comparison_panel, config_panel, input_panel, overview_panel, results_table,
    rsu_panel, transition_panel,
};
use crate::ui::panels::input_panel::InputPanelState;

/// Result bundle returned by the simulation background thread.
type SimBundle = (SimResults, RsuEngine, Option<MarcoPoloResults>);

/// Embed the Noto Sans JP font from the project root and prioritize it for
/// the Proportional family so Japanese characters render in tooltips and
/// table cells (e.g. "NHI Spike", "公的年金等控除").
fn install_noto_sans_jp(ctx: &egui::Context) {
    const NOTO_JP: &[u8] = include_bytes!("../../NotoSansJP-Regular.otf");
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "NotoSansJP".to_owned(),
        FontData::from_static(NOTO_JP),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "NotoSansJP".to_owned());
    fonts
        .families
        .entry(FontFamily::Monospace)
        .or_default()
        .push("NotoSansJP".to_owned());
    ctx.set_fonts(fonts);
}

/// The active tab in the results pane.
#[derive(Default, PartialEq, Eq)]
enum Tab {
    #[default]
    Overview,
    Table,
    Charts,
    Rsu,
    Transition,
    InputConfig,
    Comparison,
}

/// Application state and UI logic.
pub struct RetirementApp {
    scenario_path:      Option<String>,
    scenario_name:      Option<String>,
    results:            Option<SimResults>,
    rsu_engine:         Option<RsuEngine>,
    marco_polo_results: Option<MarcoPoloResults>,
    active_tab:         Tab,
    status:             String,
    result_rx:          Option<mpsc::Receiver<Result<SimBundle, String>>>,
    running:            bool,
    /// Editable input state for the Input Configuration tab (Baseline scenario).
    input_state:        InputPanelState,
    /// V6.6: track whether Noto Sans JP has been installed on the egui context.
    fonts_installed:    bool,
    // ── Dual-scenario comparison ─────────────────────────────────────────────
    comparison_path:    Option<String>,
    comparison_name:    Option<String>,
    comparison_results: Option<SimResults>,
    comparison_rx:      Option<mpsc::Receiver<Result<SimResults, String>>>,
    comparison_running: bool,
}

impl Default for RetirementApp {
    fn default() -> Self {
        Self {
            scenario_path:      None,
            scenario_name:      None,
            results:            None,
            rsu_engine:         None,
            marco_polo_results: None,
            active_tab:         Tab::default(),
            status:             "Ready. Open a scenario JSON file to begin.".into(),
            result_rx:          None,
            running:            false,
            input_state:        InputPanelState::default(),
            fonts_installed:    false,
            comparison_path:    None,
            comparison_name:    None,
            comparison_results: None,
            comparison_rx:      None,
            comparison_running: false,
        }
    }
}

impl eframe::App for RetirementApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // ── First-frame font setup: register Noto Sans JP so Japanese glyphs ──
        // (NHI Spike など) render correctly in tooltips, labels, and tables.
        if !self.fonts_installed {
            install_noto_sans_jp(ctx);
            self.fonts_installed = true;
        }

        // ── Handle reload request from the Input Configuration panel ───────────
        if let Some(path) = self.input_state.reload_path.take() {
            self.load_scenario_from_path(path);
        }

        // ── Poll background threads ────────────────────────────────────────────
        self.poll_simulation_result(ctx);
        self.poll_comparison_result(ctx);

        // ── Top toolbar ────────────────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("🏦 Retirement Calculator");
                ui.add_space(20.0);

                if ui.button("📂 Open Scenario").clicked() {
                    self.open_file_dialog();
                }

                if ui.button("📂 Open Comparison").clicked() {
                    self.open_comparison_dialog();
                }

                ui.add_space(8.0);

                let has_errors = !self.input_state.validation_errors().is_empty();
                let run_enabled = self.scenario_path.is_some() && !self.running && !has_errors;
                let run_btn = ui.add_enabled(
                    run_enabled,
                    egui::Button::new(if self.running { "⏳ Running…" } else { "▶ Run Baseline" }),
                );
                if run_btn.clicked() {
                    self.start_simulation(ctx.clone());
                }

                if self.comparison_path.is_some() {
                    ui.add_space(4.0);
                    let cmp_enabled = !self.comparison_running;
                    let cmp_btn = ui.add_enabled(
                        cmp_enabled,
                        egui::Button::new(
                            if self.comparison_running { "⏳ Comparing…" } else { "▶ Run Comparison" },
                        ),
                    );
                    if cmp_btn.clicked() {
                        self.start_comparison(ctx.clone());
                    }
                    if let Some(name) = &self.comparison_name {
                        ui.label(
                            RichText::new(format!("vs {}", name))
                                .small()
                                .color(Color32::from_rgb(220, 160, 60)),
                        );
                    }
                }

                ui.add_space(16.0);

                let color = if self.status.contains("Error") || self.status.contains("failed") {
                    Color32::RED
                } else if self.status.contains("complete") || self.status.contains("ready") {
                    Color32::GREEN
                } else {
                    Color32::GRAY
                };
                ui.label(RichText::new(&self.status).color(color).small());
            });
        });

        // ── Left config panel ──────────────────────────────────────────────────
        egui::SidePanel::left("config_panel")
            .default_width(220.0)
            .resizable(true)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    config_panel::show(ui, &self.scenario_name);
                });
            });

        // ── Central results panel (tabbed) ─────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.active_tab, Tab::Overview,    "📊 Overview");
                ui.selectable_value(&mut self.active_tab, Tab::Table,       "📋 Annual Table");
                ui.selectable_value(&mut self.active_tab, Tab::Charts,      "📈 Charts");
                ui.selectable_value(&mut self.active_tab, Tab::Rsu,         "🗓 RSU Schedule");
                ui.selectable_value(&mut self.active_tab, Tab::Transition,  "🔄 Transition");
                ui.selectable_value(&mut self.active_tab, Tab::InputConfig, "⚙ Input Config");
                ui.selectable_value(&mut self.active_tab, Tab::Comparison,  "🔀 Compare");
            });
            ui.separator();

            egui::ScrollArea::vertical().id_salt("central_panel_scroll").show(ui, |ui| {
                match self.active_tab {
                    Tab::Comparison => comparison_panel::show(
                        ui,
                        &self.results,
                        &self.comparison_results,
                        &self.marco_polo_results,
                    ),
                    _ => {
                        ui.push_id("baseline_view", |ui| {
                            match self.active_tab {
                                Tab::Overview    => overview_panel::show(ui, &self.results, &self.rsu_engine),
                                Tab::Table       => results_table::show(ui, &self.results),
                                Tab::Charts      => chart_panel::show(ui, &self.results),
                                Tab::Rsu         => rsu_panel::show(ui, &self.rsu_engine, &self.results),
                                Tab::Transition  => transition_panel::show(ui, &self.results),
                                Tab::InputConfig => input_panel::show(ui, &mut self.input_state),
                                Tab::Comparison  => unreachable!(),
                            }
                        });
                    }
                }
            });
        });
    }
}

impl RetirementApp {
    fn open_file_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON Scenario", &["json"])
            .set_title("Open Baseline Scenario")
            .pick_file()
        {
            let path_str = path.to_string_lossy().to_string();
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_str.clone());
            self.load_scenario_from_path(path_str);
            self.scenario_name = Some(name);
        }
    }

    fn open_comparison_dialog(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("JSON Scenario", &["json"])
            .set_title("Open Comparison Scenario")
            .pick_file()
        {
            let path_str = path.to_string_lossy().to_string();
            let name = path.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| path_str.clone());
            self.comparison_path    = Some(path_str);
            self.comparison_name    = Some(name);
            self.comparison_results = None;
            self.status = "Comparison scenario loaded. Click '▶ Run Comparison' to execute.".into();
        }
    }

    fn load_scenario_from_path(&mut self, path: String) {
        let name = std::path::Path::new(&path)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        if let Ok(raw) = std::fs::read_to_string(&path) {
            let clean: String = raw
                .lines()
                .map(|line| {
                    let t = line.trim_start();
                    if t.starts_with("//") || t.starts_with('#') { "" } else { line }
                })
                .collect::<Vec<_>>()
                .join("\n");

            if let Ok(json) = serde_json::from_str(&clean) {
                self.input_state = InputPanelState::from_json(&json, &path);
            }
        }

        self.scenario_path  = Some(path.clone());
        self.scenario_name  = Some(name);
        self.status = format!("Scenario loaded: {}", path);
        self.results        = None;
        self.rsu_engine     = None;
        self.marco_polo_results = None;
    }

    fn start_simulation(&mut self, ctx: egui::Context) {
        let path = match &self.scenario_path {
            Some(p) => p.clone(),
            None => return,
        };

        self.running = true;
        self.status  = "Running simulation…".into();
        self.results = None;
        self.marco_polo_results = None;

        // Extract Marco Polo parameters from input state before spawning thread.
        let marco_polo_enabled = self.input_state.marco_polo_enabled;
        let mp_params: Option<MarcoPoloInput> = if marco_polo_enabled {
            Some(self.build_marco_polo_input())
        } else {
            None
        };

        let (tx, rx) = mpsc::channel();
        self.result_rx = Some(rx);

        std::thread::spawn(move || {
            let result: Result<SimBundle, String> = (|| {
                let loaded = load_scenario(&path)
                    .map_err(|e| format!("Load error: {}", e))?;

                let rsu_engine = RsuEngine::new(
                    loaded.config.rsu_awards.clone(),
                    Some(loaded.config.retirement_date),
                );

                let controller = SimulationController::new(loaded.config, loaded.accounts);
                let sim_results = controller.run();

                let marco_polo = mp_params.map(|mut mp| {
                    // Use post-retirement net cashflow from first available year.
                    if let Some(snap) = sim_results.annual_summary.first() {
                        let net_usd = snap.total_inc_net_usd - (snap.total_exp_jpy / snap.usd_jpy);
                        mp.annual_net_cashflow_usd = net_usd;
                    }
                    monte_carlo::run_marco_polo(&mp, DEFAULT_ITERATIONS)
                });

                Ok((sim_results, rsu_engine, marco_polo))
            })();

            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    fn start_comparison(&mut self, ctx: egui::Context) {
        let path = match &self.comparison_path {
            Some(p) => p.clone(),
            None => return,
        };

        self.comparison_running = true;
        self.comparison_results = None;
        self.status = "Running comparison simulation…".into();

        let (tx, rx) = mpsc::channel();
        self.comparison_rx = Some(rx);

        std::thread::spawn(move || {
            let result: Result<SimResults, String> = (|| {
                let loaded = load_scenario(&path)
                    .map_err(|e| format!("Load error: {}", e))?;
                let controller = SimulationController::new(loaded.config, loaded.accounts);
                Ok(controller.run())
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    /// Build a `MarcoPoloInput` from the current `input_state` positions.
    fn build_marco_polo_input(&self) -> MarcoPoloInput {
        let mut total_value = 0.0_f64;
        let mut weighted_mean = 0.0_f64;
        let mut weighted_vol  = 0.0_f64;

        for acct in &self.input_state.accounts {
            if acct.account_type == "DC Plan" {
                // DC plan: use dc_growth_rate and dc_volatility (always 15% default)
                let monthly_jpy: f64 = acct.dc_monthly_jpy.trim().parse().unwrap_or(0.0);
                let fx = self.input_state.usd_jpy_rate.trim().parse::<f64>().unwrap_or(145.0);
                let val = monthly_jpy / fx * 12.0; // approximate annual contribution as proxy value
                let g: f64 = acct.dc_growth_rate.trim().parse::<f64>().unwrap_or(8.0) / 100.0;
                let v: f64 = acct.dc_volatility.trim().parse::<f64>().unwrap_or(15.0) / 100.0;
                if val > 0.0 {
                    total_value   += val;
                    weighted_mean += val * g;
                    weighted_vol  += val * v;
                }
                continue;
            }
            for pos in &acct.positions {
                let qty: f64 = pos.units.trim().parse().unwrap_or(0.0);
                let price: f64 = pos.unit_value.trim().parse().unwrap_or_else(|_| {
                    crate::engine::market_data::MarketDataService::fallback_price(&pos.ticker)
                });
                let val = qty * price;
                let g: f64 = pos.growth_pct.trim().parse::<f64>().unwrap_or(7.0) / 100.0;
                let v: f64 = pos.volatility_pct.trim().parse::<f64>()
                    .unwrap_or(monte_carlo::DEFAULT_TICKER_VOL * 100.0) / 100.0;
                if val > 0.0 {
                    total_value   += val;
                    weighted_mean += val * g;
                    weighted_vol  += val * v;
                }
            }
        }

        let mean = if total_value > 0.0 { weighted_mean / total_value } else { 0.07 };
        let vol  = if total_value > 0.0 { weighted_vol  / total_value } else { 0.18 };

        // Parse simulation years from state.
        let start_year = self.input_state.start_date.get(..4)
            .and_then(|y| y.parse::<i32>().ok())
            .unwrap_or(2025);
        let end_year = self.input_state.end_date.get(..4)
            .and_then(|y| y.parse::<i32>().ok())
            .unwrap_or(2065);

        MarcoPoloInput {
            start_year,
            end_year,
            initial_value_usd: total_value,
            annual_mean_return: mean,
            annual_volatility: vol,
            annual_net_cashflow_usd: 0.0, // overridden in thread after main sim
            seed: None,
            fx_stochastic: None,
            asset_classes: None,        // Stage 08: TODO - wire from input_state
            correlation_matrix: None,   // Stage 08: TODO - wire from input_state
        }
    }

    fn poll_simulation_result(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.result_rx {
            match rx.try_recv() {
                Ok(Ok((results, rsu_engine, marco_polo))) => {
                    let year_count = results.annual_summary.len();
                    let gap_count  = results.gap_warnings.len();
                    reporter::write_reports(&results, &rsu_engine);
                    self.results            = Some(results);
                    self.rsu_engine         = Some(rsu_engine);
                    self.marco_polo_results = marco_polo;
                    self.running    = false;
                    self.result_rx  = None;
                    self.status = format!(
                        "✅ Simulation complete — {} years, {} warnings | Reports → output/",
                        year_count, gap_count
                    );
                    self.active_tab = Tab::Overview;
                    ctx.request_repaint();
                }
                Ok(Err(e)) => {
                    self.status    = format!("❌ Simulation failed: {}", e);
                    self.running   = false;
                    self.result_rx = None;
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status    = "❌ Simulation thread disconnected unexpectedly.".into();
                    self.running   = false;
                    self.result_rx = None;
                }
            }
        }
    }

    fn poll_comparison_result(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.comparison_rx {
            match rx.try_recv() {
                Ok(Ok(results)) => {
                    self.comparison_results  = Some(results);
                    self.comparison_running  = false;
                    self.comparison_rx       = None;
                    self.status = "✅ Comparison complete. View results in 🔀 Compare tab.".into();
                    self.active_tab = Tab::Comparison;
                    ctx.request_repaint();
                }
                Ok(Err(e)) => {
                    self.status             = format!("❌ Comparison failed: {}", e);
                    self.comparison_running = false;
                    self.comparison_rx      = None;
                    ctx.request_repaint();
                }
                Err(mpsc::TryRecvError::Empty) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.status             = "❌ Comparison thread disconnected.".into();
                    self.comparison_running = false;
                    self.comparison_rx      = None;
                }
            }
        }
    }
}
