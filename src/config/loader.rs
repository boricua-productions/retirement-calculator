use chrono::{Datelike, NaiveDate};
use log::{info, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;

use crate::engine::market_data::MarketDataService;
use crate::engine::tax::us_tax::state_tax_rate;
use crate::models::assets::{Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass, Currency, DetailedReturnProfile, DividendCurrency};
use crate::models::config::{AccumulationRule, Config, Dependent, FamilyUnit, FXShockEvent, InvestmentLocation, MilitaryRetiredConfig, NhiCalculatedRates, NhiModel, RecessionEvent, TaxProtocol, TaxRules, UsTaxStrategy, VaDependentStatus, VaRates, WaterfallStrategy, WithdrawalStrategy};
use crate::models::expense::ExpenseRule;
use crate::models::rsu::{RsuAward, VestingCadence};

/// Errors that can occur during scenario loading.
#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Logic(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Io(e)    => write!(f, "IO error: {}", e),
            LoadError::Json(e)  => write!(f, "JSON parse error: {}", e),
            LoadError::Logic(s) => write!(f, "Config error: {}", s),
        }
    }
}

/// A fully parsed scenario ready to hand off to `SimulationController`.
pub struct LoadedScenario {
    pub config: Config,
    pub accounts: HashMap<String, Account>,
}

/// V7.6 — Parse `info["asset_class"]` snake_case string into an `AssetClass`.
fn parse_asset_class(info: &Value) -> AssetClass {
    match info["asset_class"].as_str().unwrap_or("stock") {
        "etf" | "ETF" | "Etf"                   => AssetClass::Etf,
        "mutual_fund" | "MutualFund" | "mutual" => AssetClass::MutualFund,
        "other" | "Other"                       => AssetClass::Other,
        _                                       => AssetClass::Stock,
    }
}

/// V7.6 — Parse `info["return_profile"]` object into a `DetailedReturnProfile`.
/// Returns `None` if the field is absent or not an object (engine falls back to
/// the legacy single-yield model).
fn parse_return_profile(info: &Value) -> Option<DetailedReturnProfile> {
    let p = info.get("return_profile")?;
    if !p.is_object() { return None; }
    let f = |k: &str| p[k].as_f64().unwrap_or(0.0);
    Some(DetailedReturnProfile {
        cap_growth:     f("cap_growth"),
        nav_growth:     f("nav_growth"),
        dividend_yield: f("dividend_yield"),
        interest_yield: f("interest_yield"),
        cap_gains_dist: f("cap_gains_dist"),
        special_dist:   f("special_dist"),
        roc:            f("roc"),
        expense_ratio:  f("expense_ratio"),
    })
}

/// Parse a JSON scenario file into a `LoadedScenario`.
///
/// Strips `//` and `#` comments from the file before parsing.
pub fn load_scenario(path: &str) -> Result<LoadedScenario, LoadError> {
    let raw = fs::read_to_string(path).map_err(LoadError::Io)?;

    let clean: String = raw.lines().map(|line| {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('#') {
            ""
        } else {
            line
        }
    }).collect::<Vec<_>>().join("\n");

    let data: Value = serde_json::from_str(&clean).map_err(LoadError::Json)?;
    let sets = &data["simulation_settings"];

    // ── Helper closures ────────────────────────────────────────────────────────
    let get_date = |key: &str, default: &str| -> NaiveDate {
        let s = sets[key].as_str().unwrap_or(default);
        NaiveDate::parse_from_str(s, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::parse_from_str(default, "%Y-%m-%d").unwrap())
    };

    let get_f64 = |key: &str, default: f64| -> f64 {
        match &sets[key] {
            Value::Number(n) => n.as_f64().unwrap_or(default),
            Value::Array(arr) => arr.first().and_then(|v| v.as_f64()).unwrap_or(default),
            _ => default,
        }
    };

    let get_u32 = |key: &str, default: u32| -> u32 {
        sets[key].as_u64().unwrap_or(default as u64) as u32
    };

    let get_bool = |key: &str, default: bool| -> bool {
        sets[key].as_bool().unwrap_or(default)
    };

    let get_str = |key: &str, default: &str| -> String {
        sets[key].as_str().unwrap_or(default).to_string()
    };

    // ── Dates ─────────────────────────────────────────────────────────────────
    let start_date       = get_date("start_date",       "2025-12-31");
    let end_date         = get_date("end_date",          "2080-12-31");
    let retirement_date  = get_date("retirement_date",   "2031-01-01");
    let rebalance_date   = get_date("rebalance_date",    "2031-02-01");
    let birth_date       = get_date("birth_date",        "1980-01-01");
    let spouse_birth_date = get_date("spouse_birth_date", "1982-01-01");
    let child_birth_date  = get_date("child_birth_date",  "2018-09-18");

    // fers_start_date: can be specified directly OR derived from fers_start_age + birth_date.
    let fers_start_date = if sets["fers_start_date"].is_string() {
        get_date("fers_start_date", "2037-09-01")
    } else if sets["fers_start_age"].is_number() {
        let age = get_u32("fers_start_age", 62);
        add_years_naive(birth_date, age)
    } else {
        get_date("fers_start_date", "2037-09-01")
    };

    if rebalance_date < retirement_date {
        return Err(LoadError::Logic(
            "rebalance_date cannot be before retirement_date".into()
        ));
    }

    // VA child cutoff: child's 18th birthday.
    let va_child_cutoff = {
        let y = child_birth_date.year() + 18;
        NaiveDate::from_ymd_opt(y, child_birth_date.month(), child_birth_date.day())
    };

    // ── Economics ─────────────────────────────────────────────────────────────
    let usd_jpy = {
        let raw = get_f64("usd_jpy_rate", 0.0);
        if raw <= 0.0 { MarketDataService::fallback_fx_rate() } else { raw }
    };

    let inflation_us = get_f64("inflation_us_cpi",    0.028);
    let inflation_jp = get_f64("inflation_japan_cpi", 0.028);
    let ira_growth   = get_f64("roth_limit_growth",   0.03);

    // ── Growth rates ──────────────────────────────────────────────────────────
    // Determine if live market data fetching is requested.
    let fetch_live = get_bool("fetch_live_growth_rates", false);

    let mut growth_rates: HashMap<String, f64> = HashMap::new();
    if let Value::Object(map) = &sets["growth_rates_annual"] {
        for (k, v) in map {
            if let Some(rate) = v.as_f64() {
                growth_rates.insert(k.clone(), rate);
            }
        }
    }

    // ── VA disability rates ───────────────────────────────────────────────────
    let mut va_rates: HashMap<String, VaRates> = HashMap::new();
    if let Value::Object(map) = &sets["va_disability_rates"] {
        for (year_str, obj) in map {
            if year_str.starts_with('_') { continue; }
            let base  = obj["base"].as_f64().unwrap_or(0.0);
            let addon = obj["child_addon"].as_f64().unwrap_or(0.0);
            va_rates.insert(year_str.clone(), VaRates { base, child_addon: addon });
        }
    }

    // ── NHI model ─────────────────────────────────────────────────────────────
    // Legacy field kept for backward compat; NhiModel::Calculated is the new path.
    let nhi_spike_monthly_jpy = get_f64("nhi_spike_monthly_jpy",
        crate::models::constants::SimConstants::NHI_SPIKE_MONTHLY_JPY);

    let nhi_model: NhiModel = if let Value::Object(m) = &sets["nhi_model"] {
        let mode = m.get("mode").and_then(|v| v.as_str()).unwrap_or("calculated");
        if mode == "manual_override" {
            let spike   = m.get("spike_year_total_jpy")
                .and_then(|v| v.as_f64())
                .unwrap_or(nhi_spike_monthly_jpy * 12.0);
            let ongoing = m.get("ongoing_annual_total_jpy")
                .and_then(|v| v.as_f64())
                .unwrap_or(nhi_spike_monthly_jpy * 12.0);
            NhiModel::ManualOverride { spike_year_total_jpy: spike, ongoing_annual_total_jpy: ongoing }
        } else {
            // Parse each rate, defaulting to Sagamihara 2026 values.
            let def = NhiCalculatedRates::sagamihara_2026();
            NhiModel::Calculated(NhiCalculatedRates {
                medical_rate:             m.get("medical_rate").and_then(|v| v.as_f64()).unwrap_or(def.medical_rate),
                per_capita_medical:       m.get("per_capita_medical").and_then(|v| v.as_f64()).unwrap_or(def.per_capita_medical),
                cap_medical:              m.get("cap_medical").and_then(|v| v.as_f64()).unwrap_or(def.cap_medical),
                elderly_support_rate:     m.get("elderly_support_rate").and_then(|v| v.as_f64()).unwrap_or(def.elderly_support_rate),
                per_capita_support:       m.get("per_capita_support").and_then(|v| v.as_f64()).unwrap_or(def.per_capita_support),
                cap_support:              m.get("cap_support").and_then(|v| v.as_f64()).unwrap_or(def.cap_support),
                nursing_care_rate:        m.get("nursing_care_rate").and_then(|v| v.as_f64()).unwrap_or(def.nursing_care_rate),
                per_capita_nursing:       m.get("per_capita_nursing").and_then(|v| v.as_f64()).unwrap_or(def.per_capita_nursing),
                cap_nursing:              m.get("cap_nursing").and_then(|v| v.as_f64()).unwrap_or(def.cap_nursing),
                include_us_investment_income: m.get("include_us_investment_income").and_then(|v| v.as_bool()).unwrap_or(false),
            })
        }
    } else {
        // No nhi_model in JSON: default to Calculated (Sagamihara 2026).
        // Dynamic scheduling in the controller replaces the old static spike rule.
        NhiModel::Calculated(NhiCalculatedRates::sagamihara_2026())
    };

    // ── Nenkin expense rules ──────────────────────────────────────────────────
    let nenkin_monthly_jpy        = get_f64("nenkin_monthly_household_jpy", 35_020.0);
    let nenkin_baseline_annual_jpy = get_f64("nenkin_baseline_annual_jpy",  171_800.0);
    let nenkin_start = add_months(retirement_date, 1);
    let user_nenkin_end   = add_years_naive(birth_date,        60);
    let spouse_nenkin_end = add_years_naive(spouse_birth_date, 60);
    let nenkin_end = user_nenkin_end.max(spouse_nenkin_end);

    let total_annual_nenkin      = nenkin_monthly_jpy * 12.0;
    let additional_annual_nenkin = (total_annual_nenkin - nenkin_baseline_annual_jpy).max(0.0);
    let additional_nenkin_monthly = additional_annual_nenkin / 12.0;

    // NHI is scheduled dynamically by the controller (schedule_annual_nhi).
    // Only Nenkin excess (above the embedded baseline) is pre-loaded here.
    let mut expense_rules: Vec<ExpenseRule> = Vec::new();
    if additional_nenkin_monthly > 0.0 {
        expense_rules.push(ExpenseRule::new(
            "Nenkin (Excess)", additional_nenkin_monthly, nenkin_start, nenkin_end,
        ));
    }

    // ── RSU awards ────────────────────────────────────────────────────────────
    let mut rsu_awards: Vec<RsuAward> = Vec::new();
    if let Value::Array(arr) = &data["rsu_awards"] {
        for rsu in arr {
            let grant_str = rsu["grant_date"].as_str()
                .or_else(|| rsu["award_date"].as_str())
                .unwrap_or("2020-01-01");
            let grant_date = NaiveDate::parse_from_str(grant_str, "%Y-%m-%d")
                .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2020, 1, 1).unwrap());

            // Optional explicit vesting_start_date
            let vesting_start_date = rsu["vesting_start_date"].as_str()
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

            let vesting_years = match &rsu["vesting_years"] {
                Value::Array(arr) => arr.len() as u32,
                Value::Number(n)  => n.as_u64().unwrap_or(4) as u32,
                _ => 4,
            };

            let total_shares: f64 = match &rsu["total_shares"].as_f64() {
                Some(s) => *s,
                None => match &rsu["total_shares"] {
                    Value::Array(arr) => arr.iter().filter_map(|v| v.as_f64()).sum(),
                    _ => rsu["shares"].as_f64().unwrap_or(0.0),
                }
            };

            let vesting_cadence = match rsu["vesting_cadence"].as_str().unwrap_or("quarterly") {
                "monthly"  | "Monthly"  => VestingCadence::Monthly,
                "annually" | "Annually" => VestingCadence::Annually,
                _                       => VestingCadence::Quarterly,
            };

            // vesting_months: explicit list takes precedence over cadence.
            // Drop any month outside 1..=12 to prevent NaiveDate::from_ymd_opt panics downstream.
            let vesting_months: Vec<u32> = if let Value::Array(arr) = &rsu["vesting_months"] {
                arr.iter()
                    .filter_map(|v| v.as_u64().map(|n| n as u32))
                    .filter(|m| {
                        let ok = (1..=12).contains(m);
                        if !ok {
                            warn!("Ignoring invalid vesting month {} (must be 1..=12)", m);
                        }
                        ok
                    })
                    .collect()
            } else {
                vec![]
            };

            let ticker = rsu["ticker"].as_str().unwrap_or("TBD").to_string();

            let vesting_months_total: Option<u32> = rsu["vesting_months_total"]
                .as_u64().map(|m| m as u32);

            let cliff_vest_months: u32 = rsu["cliff_vest_months"]
                .as_u64().unwrap_or(0) as u32;

            rsu_awards.push(RsuAward {
                grant_date,
                vesting_start_date,
                ticker,
                total_shares,
                vesting_years,
                vesting_months_total,
                vesting_months,
                vesting_cadence,
                cliff_vest_months,
                unit_value:   rsu["unit_value"].as_f64().filter(|&p| p > 0.0),
                growth_rate:  rsu["growth_rate"].as_f64().filter(|&g| g > -0.5 && g < 1.0),
                return_profile: rsu["return_profile"].as_object().map(|obj| {
                    let f = |k: &str| obj.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    crate::models::assets::DetailedReturnProfile {
                        cap_growth:     f("cap_growth"),
                        nav_growth:     f("nav_growth"),
                        dividend_yield: f("dividend_yield"),
                        interest_yield: f("interest_yield"),
                        cap_gains_dist: f("cap_gains_dist"),
                        special_dist:   f("special_dist"),
                        roc:            f("roc"),
                        expense_ratio:  f("expense_ratio"),
                    }
                }),
            });
        }
    }

    // ── Recession events ──────────────────────────────────────────────────────
    // Supports both legacy {year, severity} and new {year, severity, duration_months, recovery_months}.
    // Bare number entries (year only) default to 20% severity, 1-month duration.
    let recession_events: Vec<RecessionEvent> =
        if let Value::Array(arr) = &sets["simulated_recessions"] {
            arr.iter().filter_map(|item| {
                let year = match item {
                    Value::Number(n) => n.as_i64()? as i32,
                    Value::Object(obj) => obj.get("year")?.as_i64()? as i32,
                    _ => return None,
                };
                let severity = item.get("severity").and_then(|v| v.as_f64()).unwrap_or(0.20);
                let duration_months = item.get("duration_months")
                    .and_then(|v| v.as_u64()).unwrap_or(1) as u32;
                let recovery_months = item.get("recovery_months")
                    .and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                Some(RecessionEvent { year, severity, duration_months, recovery_months })
            }).collect()
        } else {
            vec![]
        };

    // ── FX shock events ───────────────────────────────────────────────────────
    let fx_shock_events: Vec<FXShockEvent> =
        if let Value::Array(arr) = &sets["fx_shock_events"] {
            arr.iter().filter_map(|item| {
                let year = item.get("year")?.as_i64()? as i32;
                let target_fx = item.get("target_fx")?.as_f64()?;
                let description = item.get("description")
                    .and_then(|v| v.as_str()).unwrap_or("").to_string();
                Some(FXShockEvent { year, target_fx, description })
            }).collect()
        } else {
            vec![]
        };

    // ── Military Retired Pay ──────────────────────────────────────────────────
    let military_retired: Option<MilitaryRetiredConfig> =
        if let Value::Object(obj) = &sets["military_retired"] {
            let monthly_usd = obj.get("monthly_usd").and_then(|v| v.as_f64()).unwrap_or(0.0);
            if monthly_usd > 0.0 {
                let jurisdiction = match obj.get("jurisdiction").and_then(|v| v.as_str()).unwrap_or("both") {
                    "us_only"    | "UsOnly"    => TaxProtocol::UsOnly,
                    "japan_only" | "JapanOnly" => TaxProtocol::JapanOnly,
                    "tax_free"   | "TaxFree"   => TaxProtocol::TaxFree,
                    _                          => TaxProtocol::Both,
                };
                Some(MilitaryRetiredConfig { monthly_usd, jurisdiction })
            } else {
                None
            }
        } else if sets["military_retired_monthly_usd"].is_number() {
            let monthly_usd = get_f64("military_retired_monthly_usd", 0.0);
            if monthly_usd > 0.0 {
                Some(MilitaryRetiredConfig { monthly_usd, jurisdiction: TaxProtocol::Both })
            } else {
                None
            }
        } else {
            None
        };

    // ── Roth limit ────────────────────────────────────────────────────────────
    let roth_limit_raw = get_f64("roth_ira_annual_limit", 0.0);
    let roth_start_limit = if roth_limit_raw <= 0.0 {
        MarketDataService::roth_limit(start_date.year())
    } else {
        roth_limit_raw
    };

    // ── US Tax Strategy ───────────────────────────────────────────────────────
    let us_tax_strategy = match sets["us_tax_strategy"].as_str().unwrap_or("ftc_only") {
        "feie_and_ftc" | "FeieAndFtc" | "FEIE+FTC" => UsTaxStrategy::FeieAndFtc,
        _                                            => UsTaxStrategy::FtcOnly,
    };

    // ── VA Disability Profile ─────────────────────────────────────────────────
    let va_disability_rating = get_u32("va_disability_rating", 0);
    let va_dependent_status = match sets["va_dependent_status"].as_str().unwrap_or("vet_only") {
        "with_spouse"           | "WithSpouse"          => VaDependentStatus::WithSpouse,
        "with_spouse_and_child" | "WithSpouseAndChild"  => VaDependentStatus::WithSpouseAndChild,
        _                                                => VaDependentStatus::VetOnly,
    };

    // ── Social Security ───────────────────────────────────────────────────────
    let ss_monthly_usd       = get_f64("ss_monthly_usd",       0.0);
    let ss_start_age         = get_u32("ss_start_age",          67);

    // ── SSDI ─────────────────────────────────────────────────────────────────
    let ssdi_monthly_usd = get_f64("ssdi_monthly_usd", 0.0);

    // ── Family Unit (demographics) ────────────────────────────────────────────
    // Load explicit dependents array first; fall back to child_birth_date if absent.
    let dependents: Vec<Dependent> = if let Value::Array(arr) = &sets["dependents"] {
        arr.iter().filter_map(|d| {
            let birth_year = d["birth_year"].as_i64()? as i32;
            let is_college_student = d["is_college_student"].as_bool().unwrap_or(false);
            // V6.6: prefer full birth_date, fall back to Jan 1 of birth_year.
            let birth_date = d["birth_date"].as_str()
                .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
                .or_else(|| chrono::NaiveDate::from_ymd_opt(birth_year, 1, 1));
            Some(Dependent { birth_year, birth_date, is_college_student })
        }).collect()
    } else {
        // Backward-compat: derive one dependent from child_birth_date.
        vec![Dependent {
            birth_year: child_birth_date.year(),
            birth_date: Some(child_birth_date),
            is_college_student: false,
        }]
    };
    let family_unit = FamilyUnit {
        user_birth_year:   birth_date.year(),
        spouse_birth_year: Some(spouse_birth_date.year()),
        dependents,
    };

    // ── Nenkin pension income ─────────────────────────────────────────────────
    let nenkin_income_monthly_jpy = get_f64("nenkin_income_monthly_jpy", 0.0);
    let nenkin_income_start_age   = get_u32("nenkin_income_start_age",   65);

    // ── Japan regional tax location ───────────────────────────────────────────
    let prefecture = get_str("prefecture", "Kanagawa");
    let city       = get_str("city",       "Sagamihara");

    // ── Per-source tax jurisdictions ──────────────────────────────────────────
    let parse_protocol = |key: &str| -> TaxProtocol {
        match sets[key].as_str().unwrap_or("both") {
            "us_only"    | "UsOnly"    => TaxProtocol::UsOnly,
            "japan_only" | "JapanOnly" => TaxProtocol::JapanOnly,
            "tax_free"   | "TaxFree"   => TaxProtocol::TaxFree,
            _                          => TaxProtocol::Both,
        }
    };
    let fers_jurisdiction   = parse_protocol("fers_jurisdiction");
    let ss_jurisdiction     = parse_protocol("ss_jurisdiction");
    let nenkin_jurisdiction = parse_protocol("nenkin_jurisdiction");
    let fers_japan_local_tax_exempt = get_bool("fers_japan_local_tax_exempt", false);
    let va_smc_variant = {
        let s = get_str("va_smc_variant", "");
        if s.is_empty() { None } else { Some(s) }
    };
    let va_monthly_override = {
        let v = get_f64("va_monthly_override", -1.0);
        if v >= 0.0 { Some(v) } else { None }
    };
    let smc_monthly_override = {
        let v = get_f64("smc_monthly_override", -1.0);
        if v >= 0.0 { Some(v) } else { None }
    };

    // ── Tax jurisdiction & investment location ────────────────────────────────
    let tax_jurisdiction = match sets["tax_jurisdiction"].as_str().unwrap_or("both") {
        "us_only" | "US_ONLY" | "UsOnly"           => TaxProtocol::UsOnly,
        "japan_only" | "JAPAN_ONLY" | "JapanOnly"  => TaxProtocol::JapanOnly,
        _                                           => TaxProtocol::Both,
    };
    let investment_location = match sets["investment_location"].as_str().unwrap_or("us") {
        "japan"         | "Japan"         => InvestmentLocation::Japan,
        "international" | "International" => InvestmentLocation::International,
        _                                 => InvestmentLocation::Us,
    };

    // ── DC Plan growth rate ──────────────────────────────────────────────────
    // Priority: simulation_settings["dc_growth_rate"] → holdings.japan_dc.growth_rate → 0.08
    let dc_growth_rate = {
        let from_settings = get_f64("dc_growth_rate", -1.0);
        if from_settings >= 0.0 {
            from_settings
        } else {
            data["holdings"]["japan_dc"]["growth_rate"].as_f64().unwrap_or(0.08)
        }
    };

    // ── Tax rules: filing status + state ─────────────────────────────────────
    let filing_status = get_str("us_filing_status", "Married Filing Jointly");
    let us_state_code  = get_str("us_state_code",    "None");
    let us_state_rate  = {
        let explicit = get_f64("us_state_tax_rate", -1.0);
        if explicit >= 0.0 { explicit } else { state_tax_rate(&us_state_code) }
    };

    let tax_rules = TaxRules {
        filing_status: filing_status.clone(),
        us_state_code: us_state_code.clone(),
        us_state_rate,
        ..TaxRules::for_filing_status(&filing_status)
    };

    // ── Build Config ──────────────────────────────────────────────────────────
    let config = Config {
        start_date,
        end_date,
        retirement_date,
        rebalance_date,
        usd_jpy,
        inflation_cola: inflation_us,
        inflation_japan: inflation_jp,
        ira_limit_growth: ira_growth,
        fx_drift_enabled:  get_bool("simulate_yen_strengthening",      false),
        fx_drift_rate:     get_f64("fx_drift_rate_annual",             0.02),
        // V6.6: cadence-based JPY drift (0 cadence = legacy continuous-rate mode).
        fx_drift_cadence_months:      get_u32("fx_drift_cadence_months",      0),
        fx_drift_increase_amount_jpy: get_f64("fx_drift_increase_amount_jpy", 0.0),
        recession_enabled: get_bool("simulate_recession_at_retirement", false),
        recession_severity: get_f64("recession_severity_pct",          0.20),
        recession_events,
        fx_shock_events,
        base_expense_jpy: get_f64("base_monthly_expenses_jpy", 1_000_000.0),
        min_expense_jpy:  get_f64("min_monthly_expenses_jpy",    600_000.0),
        nhi_spike_monthly_jpy,
        nhi_model,
        war_chest_currency:  get_str("war_chest_currency",   "JPY"),
        war_chest_target_jpy: get_f64("war_chest_target_jpy", 7_000_000.0),
        war_chest_target_usd: get_f64("war_chest_target_usd",    50_000.0),
        bridge_months_target: get_u32("bridge_fund_months_target", 12),
        bridge_fund_currency: get_str("bridge_fund_currency", "USD"),
        roth_start_limit,
        roth_contribution_made_this_year: get_bool("roth_contribution_made_this_year", false),
        roth_contribution_so_far: get_f64("roth_contributions_ytd_usd", 0.0),
        dc_monthly_jpy:            get_f64("japan_dc_monthly_contribution_jpy", 45_000.0),
        dc_growth_rate,
        monthly_contribution_ticker: get_str("monthly_contribution_ticker", "VTI"),
        va_contribution_buffer_usd:  get_f64("va_contribution_buffer_usd",    800.0),
        nenkin_baseline_annual_jpy,
        growth_rates_annual: growth_rates.clone(),
        va_disability_rates: va_rates,
        fers_monthly_start: get_f64("fers_monthly_payment_usd", 794.55),
        fers_start_date,
        retirement_year_gross_income_jpy: get_f64("retirement_year_gross_income_jpy", 0.0),
        birth_date,
        spouse_birth_date,
        child_birth_date,
        va_child_cutoff_date: va_child_cutoff,
        dc_payout_start_age:  get_u32("dc_payout_start_age",  60),
        dc_payout_method:     get_str("dc_payout_method", "LUMP_SUM"),
        pre_funded_war_chest_jpy: get_f64("pre_funded_war_chest_jpy",  0.0),
        pre_funded_bridge_jpy:    get_f64("pre_funded_bridge_jpy",      0.0),
        pre_funded_bridge_usd:    get_f64("pre_funded_bridge_usd",      0.0),
        pre_funded_japan_tax_jpy: get_f64("pre_funded_japan_tax_jpy",   0.0),
        pre_funded_us_tax_usd:    get_f64("pre_funded_us_tax_usd",      0.0),
        target_vti_pct:                 get_f64("rebalance_target_vti_pct",         0.20),
        target_schd_pct:                get_f64("rebalance_target_schd_pct",        0.80),
        roth_rebalance_target_vti_pct:  get_f64("roth_rebalance_target_vti_pct",  0.50),
        roth_rebalance_target_schd_pct: get_f64("roth_rebalance_target_schd_pct", 0.50),
        enable_roth_rebalance_at_59: get_bool("enable_roth_rebalance_at_59", false),
        buy_schd_last_year:          get_bool("buy_schd_last_year",           false),
        rsu_tax_handling: get_str("rsu_tax_handling", "SALARY"),
        total_annual_compensation_usd: get_f64("total_annual_compensation_usd", 0.0),
        expense_rules,
        rsu_awards,
        tax_rules,
        tax_jurisdiction,
        investment_location,
        us_tax_strategy,
        va_disability_rating,
        va_dependent_status,
        va_monthly_override,
        smc_monthly_override,
        ss_monthly_usd,
        ss_start_age,
        ssdi_monthly_usd,
        // V6.6: marriage flag + spouse benefits (defaults preserve pre-V6.6 behaviour).
        is_married:                  sets["is_married"].as_bool()
                                          .unwrap_or(sets["spouse_birth_date"].is_string()),
        spouse_ss_monthly_usd:       get_f64("spouse_ss_monthly_usd",       0.0),
        spouse_ss_start_age:         get_u32("spouse_ss_start_age",          67),
        spouse_ss_jurisdiction:      parse_protocol("spouse_ss_jurisdiction"),
        spouse_nenkin_monthly_jpy:   get_f64("spouse_nenkin_monthly_jpy",   0.0),
        spouse_nenkin_start_age:     get_u32("spouse_nenkin_start_age",      65),
        spouse_nenkin_jurisdiction:  parse_protocol("spouse_nenkin_jurisdiction"),
        family_unit,
        nenkin_income_monthly_jpy,
        nenkin_income_start_age,
        prefecture,
        city,
        military_retired,
        fers_jurisdiction,
        ss_jurisdiction,
        nenkin_jurisdiction,
        fers_japan_local_tax_exempt,
        va_smc_variant,
        accumulation_rules: {
            let mut rules: Vec<AccumulationRule> = Vec::new();
            if let Value::Array(arr) = &sets["accumulation_rules"] {
                for item in arr {
                    let ticker  = item["ticker"].as_str().unwrap_or("").to_string();
                    let account = item["account"].as_str().unwrap_or("Taxable").to_string();
                    if ticker.is_empty() { continue; }
                    rules.push(AccumulationRule {
                        ticker,
                        account,
                        monthly_amount:      item["monthly_amount"].as_f64().unwrap_or(0.0),
                        frequency_months:    item["frequency_months"].as_u64().unwrap_or(1) as u32,
                        growth_pct_override: item["growth_pct_override"].as_f64(),
                        stop_at_retirement:  item["stop_at_retirement"].as_bool().unwrap_or(true),
                    });
                }
            }
            rules
        },
        target_allocations: {
            let mut allocs: HashMap<String, f64> = HashMap::new();
            if let Value::Object(map) = &sets["target_allocations"] {
                for (k, v) in map {
                    if let Some(w) = v.as_f64() {
                        allocs.insert(k.clone(), w);
                    }
                }
            }
            allocs
        },
        rebalance_enabled:          get_bool("rebalance_enabled",           false),
        rebalance_frequency_months: get_u32("rebalance_frequency_months",   12),

        // ── V7.0: top-level state-tax dial + withdrawal strategy ─────────────
        us_state_tax_rate: us_state_rate,
        withdrawal_strategy: match sets["withdrawal_strategy"].as_str().unwrap_or("total_return") {
            "dividend_only" | "DividendOnly" => WithdrawalStrategy::DividendOnly,
            "hybrid"        | "Hybrid"       => WithdrawalStrategy::Hybrid,
            _                                 => WithdrawalStrategy::TotalReturn,
        },

        // ── V7.1: spending waterfall + FX spread penalty ──────────────────────
        withdrawal_waterfall: match sets["withdrawal_waterfall"].as_str().unwrap_or("defensive") {
            "cautious" | "Cautious" => WaterfallStrategy::Cautious,
            _                        => WaterfallStrategy::Defensive,
        },
        fx_spread_penalty: {
            let v = get_f64("fx_spread_penalty", -1.0);
            if v >= 0.0 { v } else { 0.005 }
        },

        // ── V7.3: Education & Family Engine ──────────────────────────────────
        withdrawal_regime: match sets["withdrawal_regime"].as_str().unwrap_or("shielded") {
            "dynamic" | "Dynamic" => crate::models::config::WithdrawalRegime::Dynamic,
            _                       => crate::models::config::WithdrawalRegime::Shielded,
        },
        edu_savings_jpy_monthly: get_f64("edu_savings_jpy_monthly", 0.0),
        jido_teate_enabled: get_bool("jido_teate_enabled", true),

        // ── V7.5: Exit Tax Monitor ────────────────────────────────────────────
        japan_residency_start_date: sets["japan_residency_start_date"]
            .as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
        exit_tax_include_tax_advantaged: get_bool("exit_tax_include_tax_advantaged", true),

        // ── V7.5: Tier 9 Gift Sink ────────────────────────────────────────────
        annual_gift_jpy_per_recipient: get_f64("annual_gift_jpy_per_recipient", 0.0),
        gift_recipient_count:          get_u32("gift_recipient_count",           0),
        us_gift_exclusion_usd:         get_f64("us_gift_exclusion_usd",     19_000.0),

        // ── V7.5: Tax-Loss Harvesting ─────────────────────────────────────────
        tlh_enabled:       get_bool("tlh_enabled",      false),
        tlh_active_months: {
            if let Value::Array(arr) = &sets["tlh_active_months"] {
                arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect()
            } else {
                vec![11, 12]
            }
        },
        tlh_min_loss_usd: get_f64("tlh_min_loss_usd", 500.0),
    };

    // ── Manual price overrides ────────────────────────────────────────────────
    let mut manual_prices: HashMap<String, f64> = HashMap::new();
    if let Value::Object(map) = &data["market_prices_usd"] {
        for (k, v) in map {
            if k.starts_with('_') { continue; }
            if let Some(p) = v.as_f64() {
                manual_prices.insert(k.clone(), p);
            }
        }
    }

    // ── Build Accounts ────────────────────────────────────────────────────────
    let default_growth_tickers: std::collections::HashSet<&str> =
        ["QQQM", "MSFT", "PANW", "NVDA"].iter().copied().collect();

    let accounts = {
        let mut map: HashMap<String, Account> = HashMap::new();

        // Helper: resolve growth rate for a ticker
        let resolve_growth = |ticker: &str, explicit_rate: Option<f64>| -> f64 {
            if let Some(r) = explicit_rate {
                return r;
            }
            if fetch_live {
                info!("[Loader] Fetching 10y CAGR for {}…", ticker);
                let live = MarketDataService::fetch_10y_cagr(ticker);
                return live;
            }
            growth_rates.get(ticker).copied()
                .unwrap_or_else(|| MarketDataService::fallback_growth(ticker))
        };

        // ── Primary Taxable account ───────────────────────────────────────────
        let mut taxable = Account::new_with_meta(
            "Taxable", Currency::Usd,
            AccountLocation::Us, AccountJurisdiction::Both,
        );
        if let Value::Object(h) = &data["holdings"]["taxable"] {
            for (ticker, info) in h {
                if ticker.starts_with("//") || ticker.starts_with('_') { continue; }
                if !info.is_object() { continue; }
                let qty      = info["qty"].as_f64().unwrap_or(0.0);
                let avg_cost = info["avg_cost"].as_f64().unwrap_or(0.0);
                let basis = if avg_cost > 0.0 {
                    qty * avg_cost
                } else {
                    info["basis"].as_f64().unwrap_or(0.0)
                };
                let yield_rate   = info["yield_rate"].as_f64()
                    .unwrap_or_else(|| MarketDataService::fallback_yield(ticker));
                let custom_rate  = info["growth_rate"].as_f64();
                let growth_rate  = resolve_growth(ticker, custom_rate);
                let price = manual_prices.get(ticker.as_str()).copied()
                    .filter(|&p| p > 0.0)
                    .unwrap_or_else(|| MarketDataService::fallback_price(ticker));
                let category = if info["category"].as_str() == Some("GROWTH")
                    || default_growth_tickers.contains(ticker.as_str())
                { AssetCategory::Growth } else { AssetCategory::Income };
                let drip     = info["drip_enabled"].as_bool().unwrap_or(true);
                let reinvest = info["dividend_reinvest_target"].as_str().map(|s| s.to_string());
                let custom_growth_rate = info["custom_growth_rate"].as_f64();

                let avg_jpy_basis_per_share = {
                    let explicit = info["avg_purchase_price_jpy"].as_f64().unwrap_or(0.0);
                    if explicit > 0.0 { explicit } else { avg_cost * usd_jpy }
                };
                // V7.1: lumpy-dividend schedule and currency.
                let dividend_months: Vec<u32> = if let Value::Array(arr) = &info["dividend_months"] {
                    arr.iter().filter_map(|v| v.as_u64().map(|n| n as u32))
                        .filter(|m| (1..=12).contains(m))
                        .collect()
                } else {
                    vec![3, 6, 9, 12]
                };
                let dividend_currency = match info["dividend_currency"].as_str().unwrap_or("usd") {
                    "jpy" | "Jpy" | "JPY" => DividendCurrency::Jpy,
                    _                     => DividendCurrency::Usd,
                };
                let pfic_regime = match info["pfic_regime"].as_str().unwrap_or("not_pfic") {
                    "mtm" => crate::models::assets::PficRegime::Mtm,
                    "excess_distribution" => crate::models::assets::PficRegime::ExcessDistribution,
                    _ => crate::models::assets::PficRegime::NotPfic,
                };
                let mut asset = Asset {
                    ticker: ticker.clone(),
                    price, yield_rate, growth_rate,
                    currency: Currency::Usd,
                    category, drip_enabled: drip,
                    dividend_reinvest_target: reinvest,
                    custom_growth_rate,
                    avg_jpy_basis_per_share,
                    dividend_months,
                    dividend_currency,
                    pfic_regime,
                    pfic_prior_year_fmv_per_share: 0.0,
                    asset_class: parse_asset_class(info),
                    return_profile: parse_return_profile(info),
                    lots: Vec::new(),
                };
                asset.add_lot(start_date, qty, basis);
                taxable.assets.insert(ticker.clone(), asset);
            }
        }
        map.insert("Taxable".into(), taxable);

        // ── Additional brokerage accounts ─────────────────────────────────────
        if let Value::Array(brok_arr) = &data["brokerage_accounts"] {
            for brok in brok_arr {
                let acc_name = brok["name"].as_str()
                    .map(|n| format!("Brokerage_{}", n.replace(' ', "_")))
                    .unwrap_or_else(|| format!("Brokerage_{}", map.len()));

                let acc_jurisdiction = match brok["tax_jurisdiction"].as_str().unwrap_or("both") {
                    "us" | "US"       => AccountJurisdiction::Us,
                    "japan" | "Japan" => AccountJurisdiction::Japan,
                    "none" | "None"   => AccountJurisdiction::None,
                    _                 => AccountJurisdiction::Both,
                };
                let acc_location = match brok["location"].as_str().unwrap_or("us") {
                    "japan" | "Japan"           => AccountLocation::Japan,
                    "both"  | "Both"            => AccountLocation::Both,
                    "none"  | "None"            => AccountLocation::None,
                    _                           => AccountLocation::Us,
                };

                let mut account = Account::new_with_meta(
                    acc_name.clone(), Currency::Usd, acc_location, acc_jurisdiction,
                );

                if let Value::Object(holdings) = &brok["holdings"] {
                    for (ticker, info) in holdings {
                        if ticker.starts_with('_') { continue; }
                        if !info.is_object() { continue; }
                        let qty      = info["qty"].as_f64().unwrap_or(0.0);
                        let avg_cost = info["avg_cost"].as_f64().unwrap_or(0.0);
                        let basis    = qty * avg_cost;
                        let yield_rate  = info["yield_rate"].as_f64()
                            .unwrap_or_else(|| MarketDataService::fallback_yield(ticker));
                        let custom_rate = info["growth_rate"].as_f64();
                        let growth_rate = resolve_growth(ticker, custom_rate);
                        let price = manual_prices.get(ticker.as_str()).copied()
                            .filter(|&p| p > 0.0)
                            .unwrap_or_else(|| MarketDataService::fallback_price(ticker));
                        let custom_growth_rate = info["custom_growth_rate"].as_f64();

                        let avg_jpy_basis_per_share = {
                            let explicit = info["avg_purchase_price_jpy"].as_f64().unwrap_or(0.0);
                            if explicit > 0.0 { explicit } else { avg_cost * usd_jpy }
                        };
                        let div_months: Vec<u32> = info["dividend_months"]
                            .as_array()
                            .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                            .unwrap_or_else(|| vec![3, 6, 9, 12]);
                        let div_currency = match info["dividend_currency"].as_str() {
                            Some("jpy") | Some("JPY") => DividendCurrency::Jpy,
                            _ => DividendCurrency::Usd,
                        };
                        let pfic_regime_b = match info["pfic_regime"].as_str().unwrap_or("not_pfic") {
                            "mtm" => crate::models::assets::PficRegime::Mtm,
                            "excess_distribution" => crate::models::assets::PficRegime::ExcessDistribution,
                            _ => crate::models::assets::PficRegime::NotPfic,
                        };
                        let mut asset = Asset {
                            ticker: ticker.clone(),
                            price, yield_rate, growth_rate,
                            currency: Currency::Usd,
                            category: AssetCategory::Income,
                            drip_enabled: true,
                            dividend_reinvest_target: None,
                            custom_growth_rate,
                            avg_jpy_basis_per_share,
                            dividend_months: div_months,
                            dividend_currency: div_currency,
                            pfic_regime: pfic_regime_b,
                            pfic_prior_year_fmv_per_share: 0.0,
                            asset_class: parse_asset_class(info),
                            return_profile: parse_return_profile(info),
                            lots: Vec::new(),
                        };
                        asset.add_lot(start_date, qty, basis);
                        account.assets.insert(ticker.clone(), asset);
                    }
                }

                map.insert(acc_name, account);
            }
        }

        // ── Roth IRA ─────────────────────────────────────────────────────────
        let mut roth = Account::new_with_meta(
            "Roth", Currency::Usd,
            AccountLocation::Us, AccountJurisdiction::Us,
        );
        if let Value::Object(h) = &data["holdings"]["roth_ira"] {
            for (ticker, info) in h {
                if ticker.starts_with("//") || ticker.starts_with('_') { continue; }
                if !info.is_object() { continue; }
                let qty      = info["qty"].as_f64().unwrap_or(0.0);
                let avg_cost = info["avg_cost"].as_f64().unwrap_or(0.0);
                let basis = if avg_cost > 0.0 { qty * avg_cost }
                    else { info["basis"].as_f64().unwrap_or(0.0) };
                let yield_rate  = info["yield_rate"].as_f64()
                    .unwrap_or_else(|| MarketDataService::fallback_yield(ticker));
                let custom_rate = info["growth_rate"].as_f64();
                let growth_rate = resolve_growth(ticker, custom_rate);
                let price = manual_prices.get(ticker.as_str()).copied()
                    .filter(|&p| p > 0.0)
                    .unwrap_or_else(|| MarketDataService::fallback_price(ticker));
                let drip     = info["drip_enabled"].as_bool().unwrap_or(true);
                let reinvest = info["dividend_reinvest_target"].as_str().map(|s| s.to_string());
                let custom_growth_rate = info["custom_growth_rate"].as_f64();

                let avg_jpy_basis_per_share = avg_cost * usd_jpy;
                let div_months: Vec<u32> = info["dividend_months"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_u64().map(|n| n as u32)).collect())
                    .unwrap_or_else(|| vec![3, 6, 9, 12]);
                let div_currency = match info["dividend_currency"].as_str() {
                    Some("jpy") | Some("JPY") => DividendCurrency::Jpy,
                    _ => DividendCurrency::Usd,
                };
                let mut asset = Asset {
                    ticker: ticker.clone(),
                    price, yield_rate, growth_rate,
                    currency: Currency::Usd,
                    category: AssetCategory::Income,
                    drip_enabled: drip,
                    dividend_reinvest_target: reinvest,
                    custom_growth_rate,
                    avg_jpy_basis_per_share,
                    dividend_months: div_months,
                    dividend_currency: div_currency,
                    pfic_regime: crate::models::assets::PficRegime::NotPfic,
                    pfic_prior_year_fmv_per_share: 0.0,
                    asset_class: parse_asset_class(info),
                    return_profile: parse_return_profile(info),
                    lots: Vec::new(),
                };
                asset.add_lot(start_date, qty, basis);
                roth.assets.insert(ticker.clone(), asset);
            }
        }
        map.insert("Roth".into(), roth);

        // ── Japan DC (iDeCo) ──────────────────────────────────────────────────
        let dc_info   = &data["holdings"]["japan_dc"];
        let nav_per_10k = dc_info["nav_jpy_per_10k"].as_f64().unwrap_or(10_000.0);
        let dc_nav    = nav_per_10k / 10_000.0;
        let dc_qty    = dc_info["qty"].as_f64().unwrap_or(0.0);
        let dc_growth = dc_growth_rate;

        let mut tawara = Asset {
            ticker: "TAWARA".into(),
            price: dc_nav,
            yield_rate: 0.0,
            growth_rate: dc_growth,
            currency: Currency::Jpy,
            category: AssetCategory::Income,
            drip_enabled: true,
            dividend_reinvest_target: None,
            custom_growth_rate: None,
            avg_jpy_basis_per_share: dc_nav,
            dividend_months: vec![3, 6, 9, 12],
            dividend_currency: DividendCurrency::Jpy,
            pfic_regime: crate::models::assets::PficRegime::NotPfic,
            pfic_prior_year_fmv_per_share: 0.0,
            asset_class: crate::models::assets::AssetClass::default(),
            return_profile: None,
            lots: Vec::new(),
        };
        tawara.add_lot(start_date, dc_qty, dc_qty * dc_nav);

        let mut dc = Account::new_with_meta(
            "DC", Currency::Jpy,
            AccountLocation::Japan, AccountJurisdiction::Japan,
        );
        dc.assets.insert("TAWARA".into(), tawara);
        map.insert("DC".into(), dc);

        map
    };

    Ok(LoadedScenario { config, accounts })
}

/// Add `months` to a NaiveDate, handling month-end clamping.
fn add_months(date: NaiveDate, months: u32) -> NaiveDate {
    use chrono::Months;
    date.checked_add_months(Months::new(months)).unwrap_or(date)
}

/// Add `years` to a NaiveDate, clamping on Feb 29 → Feb 28.
fn add_years_naive(date: NaiveDate, years: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(date.year() + years as i32, date.month(), date.day())
        .or_else(|| NaiveDate::from_ymd_opt(date.year() + years as i32, date.month(), date.day() - 1))
        .unwrap_or(date)
}
