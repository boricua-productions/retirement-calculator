use std::time::SystemTime;

// ── Lightweight xorshift64 PRNG (no external dependency) ──────────────────────

struct Rng { state: u64 }

impl Rng {
    fn new(seed: u64) -> Self {
        Self { state: if seed == 0 { 0x9e3779b97f4a7c15 } else { seed } }
    }
    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    // Box-Muller transform → N(0, 1)
    fn normal(&mut self) -> f64 {
        let u1 = self.next_f64().max(1e-12);
        let u2 = self.next_f64();
        (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos()
    }
}

// ── Public types ──────────────────────────────────────────────────────────────

/// V7.5 — Optional stochastic FX parameters for USD/JPY path simulation.
pub struct FxStochasticParams {
    /// Starting USD/JPY rate (e.g. 145.0).
    pub initial_fx: f64,
    /// Annual mean drift of the FX path (e.g. 0.02 → yen weakens 2%/yr on average).
    pub annual_mean_drift: f64,
    /// Annual volatility of the FX path (e.g. 0.10 for 10% σ).
    pub annual_volatility: f64,
}

/// Input parameters for the Marco Polo Monte Carlo engine.
pub struct MarcoPoloInput {
    pub start_year: i32,
    pub end_year: i32,
    /// Combined initial portfolio value in USD (sum of all positions).
    pub initial_value_usd: f64,
    /// Weighted-average annual expected return (CAGR) across all positions.
    pub annual_mean_return: f64,
    /// Weighted-average annual volatility (std dev of return) across all positions.
    pub annual_volatility: f64,
    /// Deterministic annual net cash flow added each year (income − expenses).
    /// Positive = inflow, negative = draw-down.
    pub annual_net_cashflow_usd: f64,
    /// Optional fixed RNG seed for reproducible runs. `None` uses wall-clock entropy.
    pub seed: Option<u64>,
    /// V7.5 — Optional stochastic FX. When Some, each iteration draws an independent
    /// USD/JPY path and output percentiles are expressed in both USD and JPY terms.
    pub fx_stochastic: Option<FxStochasticParams>,
}

/// Percentile trajectories produced by the Marco Polo engine.
#[derive(Clone)]
pub struct MarcoPoloResults {
    pub years: Vec<i32>,
    /// 10th-percentile net worth at each simulated year (worst-case).
    pub p10: Vec<f64>,
    /// 50th-percentile net worth (median).
    pub p50: Vec<f64>,
    /// 90th-percentile net worth (best-case).
    pub p90: Vec<f64>,
    pub iterations: usize,
    pub mean_return: f64,
    pub volatility: f64,
    /// V7.5 — JPY-denominated percentiles when stochastic FX is enabled. Empty otherwise.
    pub p10_jpy: Vec<f64>,
    pub p50_jpy: Vec<f64>,
    pub p90_jpy: Vec<f64>,
}

pub const DEFAULT_ITERATIONS: usize = 1_000;

/// Default annual volatility for individual equity tickers (18 %).
pub const DEFAULT_TICKER_VOL: f64 = 0.18;

// ── Engine ────────────────────────────────────────────────────────────────────

/// Run `n_iterations` Geometric Brownian Motion paths and return P10/P50/P90.
pub fn run_marco_polo(input: &MarcoPoloInput, n_iterations: usize) -> MarcoPoloResults {
    let n_years = (input.end_year - input.start_year + 1).max(0) as usize;

    let seed = input.seed.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0xdeadbeef)
    });
    let mut rng = Rng::new(seed);

    let mu    = input.annual_mean_return;
    let sigma = input.annual_volatility.max(1e-6);
    // GBM drift term: (μ − σ²/2) per year
    let drift = mu - 0.5 * sigma * sigma;

    // V7.5 — Stochastic FX: pre-compute GBM drift term for the FX path.
    let fx_params = input.fx_stochastic.as_ref();
    let fx_drift = fx_params.map(|p| p.annual_mean_drift - 0.5 * p.annual_volatility * p.annual_volatility);

    let mut all_runs_usd: Vec<Vec<f64>> = Vec::with_capacity(n_iterations);
    let mut all_runs_jpy: Vec<Vec<f64>> = Vec::with_capacity(n_iterations);

    for _ in 0..n_iterations {
        let mut portfolio = input.initial_value_usd.max(0.0);
        // Draw an independent FX path per iteration.
        let mut fx = fx_params.map(|p| p.initial_fx).unwrap_or(1.0);
        let mut traj_usd = Vec::with_capacity(n_years);
        let mut traj_jpy = Vec::with_capacity(n_years);
        for _ in 0..n_years {
            // Annual GBM step: S(t+1) = S(t) · exp(drift + σ·Z)
            let z = rng.normal();
            portfolio = (portfolio * (drift + sigma * z).exp()
                + input.annual_net_cashflow_usd)
                .max(0.0);
            // Independent FX GBM step (low correlation assumption).
            if let (Some(p), Some(fd)) = (fx_params, fx_drift) {
                let z_fx = rng.normal();
                fx *= (fd + p.annual_volatility * z_fx).exp();
            }
            traj_usd.push(portfolio);
            traj_jpy.push(portfolio * fx);
        }
        all_runs_usd.push(traj_usd);
        all_runs_jpy.push(traj_jpy);
    }

    let years: Vec<i32> = (input.start_year..=input.end_year).collect();
    let mut p10 = Vec::with_capacity(n_years);
    let mut p50 = Vec::with_capacity(n_years);
    let mut p90 = Vec::with_capacity(n_years);
    let mut p10_jpy = Vec::with_capacity(n_years);
    let mut p50_jpy = Vec::with_capacity(n_years);
    let mut p90_jpy = Vec::with_capacity(n_years);

    for yr_idx in 0..n_years {
        let mut vals: Vec<f64> = all_runs_usd.iter().map(|r| r[yr_idx]).collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let n = vals.len();
        p10.push(vals[((n as f64 * 0.10) as usize).min(n - 1)]);
        p50.push(vals[((n as f64 * 0.50) as usize).min(n - 1)]);
        p90.push(vals[((n as f64 * 0.90) as usize).min(n - 1)]);

        if fx_params.is_some() {
            let mut jpy_vals: Vec<f64> = all_runs_jpy.iter().map(|r| r[yr_idx]).collect();
            jpy_vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            p10_jpy.push(jpy_vals[((n as f64 * 0.10) as usize).min(n - 1)]);
            p50_jpy.push(jpy_vals[((n as f64 * 0.50) as usize).min(n - 1)]);
            p90_jpy.push(jpy_vals[((n as f64 * 0.90) as usize).min(n - 1)]);
        }
    }

    MarcoPoloResults {
        years,
        p10,
        p50,
        p90,
        iterations: n_iterations,
        mean_return: input.annual_mean_return,
        volatility: input.annual_volatility,
        p10_jpy,
        p50_jpy,
        p90_jpy,
    }
}
