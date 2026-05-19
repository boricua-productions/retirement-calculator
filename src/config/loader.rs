use chrono::{Datelike, NaiveDate};
use log::{info, warn};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;

use crate::engine::market_data::MarketDataService;
use crate::engine::tax::us_tax::state_tax_rate;
use crate::models::assets::{Account, AccountJurisdiction, AccountLocation, Asset, AssetCategory, AssetClass, Currency, DetailedReturnProfile, DividendCurrency};
use crate::models::config::{AccumulationRule, BufferFundingTiming, Config, Dependent, FamilyUnit, FXShockEvent, InvestmentLocation, MilitaryRetiredConfig, NhiCalculatedRates, NhiModel, RecessionEvent, SpouseProfile, TaxProtocol, TaxRules, UsTaxStrategy, VaDependentStatus, VaRates, WaterfallStrategy, WithdrawalStrategy};
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

    let get_buffer_timing = |key: &str| -> BufferFundingTiming {
        match sets[key].as_str() {
            Some("gradually_before_retirement") => BufferFundingTiming::GraduallyBeforeRetirement,
            _ => BufferFundingTiming::AtRetirement,
        }
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

    // V8.1 — Synthetic stop-rules for detailed expense categories with end dates.
    // These are regenerated on every load so the saved JSON stays clean (only the
    // human-edited expense_categories array is persisted, not the derived rules).
    {
        let cats_snapshot: Vec<crate::models::expense::ExpenseCategory> =
            parse_expense_categories(&sets["expense_categories"]);
        expense_rules.extend(synthetic_stop_rules_from_categories(&cats_snapshot, start_date, end_date));
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
                migrate_on_retirement: rsu["migrate_on_retirement"].as_bool().unwrap_or(false),
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

    // ── Spouse profile (Stage 02) ─────────────────────────────────────────────
    let spouse_profile: SpouseProfile = match sets["spouse_profile"].as_str().unwrap_or("us_person") {
        "nra_elected_to_be_treated_as_resident" | "nra_elected_mfj" =>
            SpouseProfile::NraElectedToBeTreatedAsResident,
        "nra_mfs" => SpouseProfile::NraMfs,
        "nra_head_of_household_eligible" | "nra_hoh" =>
            SpouseProfile::NraHeadOfHouseholdEligible,
        _ => SpouseProfile::UsPerson,
    };
    let spouse_japan_salary_jpy      = get_f64("spouse_japan_salary_jpy",      0.0);
    let spouse_japan_misc_income_jpy = get_f64("spouse_japan_misc_income_jpy", 0.0);

    // ── Tax rules: filing status + state ─────────────────────────────────────
    let filing_status = get_str("us_filing_status", "Married Filing Jointly");
    let us_state_code  = get_str("us_state_code",    "None");
    let us_state_rate  = {
        let explicit = get_f64("us_state_tax_rate", -1.0);
        if explicit >= 0.0 { explicit } else { state_tax_rate(&us_state_code) }
    };

    // NRA spouse profiles override the effective filing status for bracket selection.
    // Per-source jurisdiction overrides still apply on top (see controller).
    let effective_filing_status: &str = match spouse_profile {
        SpouseProfile::NraMfs                          => "Married Filing Separately",
        SpouseProfile::NraHeadOfHouseholdEligible      => "Head of Household",
        SpouseProfile::UsPerson |
        SpouseProfile::NraElectedToBeTreatedAsResident => &filing_status,
    };

    let tax_rules = TaxRules {
        filing_status: effective_filing_status.into(),
        us_state_code: us_state_code.clone(),
        us_state_rate,
        ..TaxRules::for_filing_status(effective_filing_status)
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
        expenses_detailed_mode: get_bool("expenses_detailed_mode", false),
        expense_categories:     parse_expense_categories(&sets["expense_categories"]),
        min_expense_buffer_jpy: get_f64("min_expense_buffer_jpy", 0.0),
        min_expense_buffer_pct: get_f64("min_expense_buffer_pct", 0.0),
        war_chest_enabled: get_bool("war_chest_enabled", true),
        war_chest_funding_timing: get_buffer_timing("war_chest_funding_timing"),
        war_chest_ramp_months: get_u32("war_chest_ramp_months", 24),
        war_chest_currency:  get_str("war_chest_currency",   "JPY"),
        war_chest_target_jpy: get_f64("war_chest_target_jpy", 7_000_000.0),
        war_chest_target_usd: get_f64("war_chest_target_usd",    50_000.0),
        bridge_fund_enabled: get_bool("bridge_fund_enabled", true),
        bridge_fund_funding_timing: get_buffer_timing("bridge_fund_funding_timing"),
        bridge_fund_ramp_months: get_u32("bridge_fund_ramp_months", 18),
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
        // Stage 02: NRA spouse profile
        spouse_profile,
        spouse_japan_salary_jpy,
        spouse_japan_misc_income_jpy,
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
            let mut outer: HashMap<String, HashMap<String, f64>> = HashMap::new();
            if let Value::Object(map) = &sets["target_allocations"] {
                let is_nested = map.values().next().map(|v| v.is_object()).unwrap_or(false);
                if is_nested {
                    for (account, inner_val) in map {
                        if let Value::Object(inner) = inner_val {
                            let mut inner_map = HashMap::new();
                            for (ticker, v) in inner {
                                if let Some(w) = v.as_f64() {
                                    inner_map.insert(ticker.clone(), w);
                                }
                            }
                            if !inner_map.is_empty() {
                                outer.insert(account.clone(), inner_map);
                            }
                        }
                    }
                } else {
                    // Flat form (legacy): treat all tickers as belonging to "Taxable".
                    info!("[Config] target_allocations: flat form detected, mapping to Taxable bucket");
                    let mut taxable: HashMap<String, f64> = HashMap::new();
                    for (ticker, v) in map {
                        if let Some(w) = v.as_f64() {
                            taxable.insert(ticker.clone(), w);
                        }
                    }
                    if !taxable.is_empty() {
                        outer.insert("Taxable".into(), taxable);
                    }
                }
            }
            outer
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

        // ── V7.7: Master Toggle Switches ─────────────────────────────────────
        enable_education_savings: get_bool("enable_education_savings", true),
        enable_gift_sink:         get_bool("enable_gift_sink",         true),

        // ── V7.7.2: RSU Sell-to-Cover Realism ────────────────────────────────
        rsu_sell_to_cover_realism: get_bool("rsu_sell_to_cover_realism", true),
        rsu_sell_to_cover_policy: match sets["rsu_sell_to_cover_policy"].as_str().unwrap_or("strict") {
            "permissive" | "Permissive" => crate::models::config::RsuSellToCoverPolicy::Permissive,
            _ => crate::models::config::RsuSellToCoverPolicy::Strict,
        },

        // ── Stage 03: Monthly Dependent Precision ─────────────────────────────
        monthly_dependent_precision: get_bool("monthly_dependent_precision", true),

        // ── Stage 04: Shock Application Order ────────────────────────────────
        shock_ordering: match sets["shock_ordering"].as_str().unwrap_or("depreciate_then_reprice") {
            "reprice_then_depreciate" => crate::models::config::ShockOrdering::RepriceThenDepreciate,
            "simultaneous"            => crate::models::config::ShockOrdering::Simultaneous,
            _                         => crate::models::config::ShockOrdering::DepreciateThenReprice,
        },

        // ── Stage 05: PFIC Basis Drift Monitor ───────────────────────────────
        track_pfic_basis_drift: get_bool("track_pfic_basis_drift", true),

        // ── Stage 06: Real Estate Module ─────────────────────────────────────
        enable_heloc_tier: get_bool("enable_heloc_tier", false),
        real_estate: parse_real_estate(&data),

        // ── Stage 07: Estate Planning ─────────────────────────────────────────
        enable_estate_planning: get_bool("enable_estate_planning", false),
        death_date: sets["death_date"].as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
        spouse_death_date: sets["spouse_death_date"].as_str()
            .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),

        // ── Stage 09: Cryptocurrency / Web3 Asset Handling ────────────────────
        crypto_tax_enabled: get_bool("crypto_tax_enabled", true),
        heirs: {
            let mut h: Vec<crate::models::config::Heir> = vec![];
            if let Value::Array(arr) = &sets["heirs"] {
                for item in arr {
                    if !item.is_object() { continue; }
                    let name = item["name"].as_str().unwrap_or("").to_string();
                    let birth_date = item["birth_date"].as_str()
                        .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());
                    let relationship = match item["relationship"].as_str().unwrap_or("child") {
                        "spouse" | "Spouse" => crate::models::config::HeirRelationship::Spouse,
                        "other"  | "Other"  => crate::models::config::HeirRelationship::Other,
                        _                   => crate::models::config::HeirRelationship::Child,
                    };
                    h.push(crate::models::config::Heir { name, birth_date, relationship });
                }
            }
            h
        },
        estate_planning_jurisdiction: match sets["estate_planning_jurisdiction"]
            .as_str().unwrap_or("both")
        {
            "us_only"    => TaxProtocol::UsOnly,
            "japan_only" => TaxProtocol::JapanOnly,
            _            => TaxProtocol::Both,
        },
        enable_gifting_optimiser: get_bool("enable_gifting_optimiser", false),
        // ── Stage 08 — Correlated Monte Carlo ─────────────────────────────────────
        mc_use_correlated_paths: get_bool("mc_use_correlated_paths", false),
        mc_correlation_matrix: {
            let mut matrix = HashMap::new();
            if let Value::Object(map) = &sets["mc_correlation_matrix"] {
                for (k1, v1) in map {
                    if let Value::Object(inner) = v1 {
                        let mut inner_map = HashMap::new();
                        for (k2, v2) in inner {
                            if let Some(corr) = v2.as_f64() {
                                inner_map.insert(k2.clone(), corr);
                            }
                        }
                        matrix.insert(k1.clone(), inner_map);
                    }
                }
            }
            matrix
        },

        // ── Stage 10 — Long-Term Care Insurance (Kaigo Hoken) ────────────────────
        kaigo_hoken_enabled: get_bool("kaigo_hoken_enabled", true),
        kaigo_hoken_brackets: {
            // If user provides custom brackets, parse them; otherwise None → use defaults
            if let Value::Array(arr) = &sets["kaigo_hoken_brackets"] {
                let mut brackets = Vec::new();
                for item in arr {
                    if let Value::Array(pair) = item
                        && pair.len() == 2
                        && let (Some(upper), Some(premium)) = (pair[0].as_f64(), pair[1].as_f64())
                    {
                        brackets.push((upper, premium));
                    }
                }
                if !brackets.is_empty() {
                    Some(crate::engine::tax::kaigo_hoken::KaigoHokenBrackets { brackets })
                } else {
                    None
                }
            } else {
                None
            }
        },
        kaigo_care_scenario: match sets["kaigo_care_scenario"].as_str().unwrap_or("none") {
            "low"    => crate::engine::tax::kaigo_hoken::CareScenario::Low,
            "medium" => crate::engine::tax::kaigo_hoken::CareScenario::Medium,
            "high"   => crate::engine::tax::kaigo_hoken::CareScenario::High,
            _        => crate::engine::tax::kaigo_hoken::CareScenario::None,
        },
        // V8.0 — Visa type for Exit Tax evaluation.
        primary_taxpayer_visa: match sets["primary_taxpayer_visa"].as_str().unwrap_or("table1") {
            "table2" => crate::models::config::VisaType::Table2,
            _        => crate::models::config::VisaType::Table1,
        },
        model_active_phase_resident_tax: get_bool("model_active_phase_resident_tax", false),
    };

    // ── Stage 08: Validate correlation matrix if correlated paths are enabled ────
    if config.mc_use_correlated_paths && !config.mc_correlation_matrix.is_empty() {
        // Build a CorrelationMatrix from the config to validate it
        use crate::simulation::monte_carlo::CorrelationMatrix;

        let labels: Vec<String> = config.mc_correlation_matrix.keys().cloned().collect();
        let n = labels.len();
        let mut data = vec![vec![0.0; n]; n];

        for (i, label_i) in labels.iter().enumerate() {
            data[i][i] = 1.0; // Set diagonal to 1.0
            if let Some(inner) = config.mc_correlation_matrix.get(label_i) {
                for (j, label_j) in labels.iter().enumerate() {
                    if let Some(&corr) = inner.get(label_j) {
                        data[i][j] = corr;
                    }
                }
            }
        }

        let corr_matrix = CorrelationMatrix { data, labels: labels.clone() };
        if let Err(e) = corr_matrix.validate() {
            eprintln!("⚠️  WARNING: Correlation matrix validation failed: {}", e);
            eprintln!("    The Monte Carlo engine will fall back to independent paths.");
            eprintln!("    Fix the matrix in your scenario JSON and reload.");
        }
    }

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
                    "qef" => crate::models::assets::PficRegime::Qef,
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
                    pfic_prior_year_fmv_per_share_jpy: 0.0,
                    pfic_mtm_loss_carryforward_usd: 0.0,
                    pfic_qef_election_year: info["pfic_qef_election_year"].as_i64().map(|y| y as i32),
                    asset_class: parse_asset_class(info),
                    return_profile: parse_return_profile(info),
                    crypto_staking_apr: info["crypto_staking_apr"].as_f64().unwrap_or(0.0),
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
                            "qef" => crate::models::assets::PficRegime::Qef,
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
                            pfic_prior_year_fmv_per_share_jpy: 0.0,
                            pfic_mtm_loss_carryforward_usd: 0.0,
                            pfic_qef_election_year: info["pfic_qef_election_year"].as_i64().map(|y| y as i32),
                            asset_class: parse_asset_class(info),
                            return_profile: parse_return_profile(info),
                            crypto_staking_apr: info["crypto_staking_apr"].as_f64().unwrap_or(0.0),
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
                    pfic_prior_year_fmv_per_share_jpy: 0.0,
                    pfic_mtm_loss_carryforward_usd: 0.0,
                    pfic_qef_election_year: None,
                    asset_class: parse_asset_class(info),
                    return_profile: parse_return_profile(info),
                    crypto_staking_apr: info["crypto_staking_apr"].as_f64().unwrap_or(0.0),
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
            pfic_prior_year_fmv_per_share_jpy: 0.0,
            pfic_mtm_loss_carryforward_usd: 0.0,
            pfic_qef_election_year: None,
            asset_class: crate::models::assets::AssetClass::default(),
            return_profile: None,
            crypto_staking_apr: 0.0,
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

/// Stage 06 — Parse the top-level `real_estate` array from the scenario JSON.
fn parse_real_estate(data: &Value) -> Vec<crate::models::real_estate::RealEstateHolding> {
    use crate::models::real_estate::*;

    let arr = match data.get("real_estate") {
        Some(Value::Array(a)) => a,
        _ => return Vec::new(),
    };

    let parse_date = |s: &str| -> Option<NaiveDate> {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
    };
    let f = |v: &Value, k: &str| v[k].as_f64().unwrap_or(0.0);
    let b = |v: &Value, k: &str| v[k].as_bool().unwrap_or(false);
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();

    arr.iter().filter_map(|item| {
        if !item.is_object() { return None; }

        let location = match item["location"].as_str().unwrap_or("japan") {
            "us" | "US" | "usa" => PropertyLocation::Us,
            "international"      => PropertyLocation::International,
            _                    => PropertyLocation::Japan,
        };
        let property_type = match item["property_type"].as_str().unwrap_or("primary") {
            "rental"    => PropertyType::Rental,
            "inherited" => PropertyType::Inherited,
            "vacation"  => PropertyType::Vacation,
            _           => PropertyType::Primary,
        };
        let structure_type = match item["structure_type"].as_str().unwrap_or("reinforced_concrete") {
            "wood"  => StructureType::Wood,
            "steel" => StructureType::Steel,
            "other" => StructureType::Other,
            _       => StructureType::ReinforcedConcrete,
        };

        let mortgage = if item["mortgage"].is_object() {
            let m = &item["mortgage"];
            let currency = match m["currency"].as_str().unwrap_or("jpy") {
                "usd" | "USD" => MortgageCurrency::Usd,
                _             => MortgageCurrency::Jpy,
            };
            let start_date = m["start_date"].as_str()
                .and_then(parse_date)
                .unwrap_or_else(|| NaiveDate::from_ymd_opt(2010, 1, 1).unwrap());
            Some(MortgageTerms {
                original_principal: f(m, "original_principal"),
                annual_rate:        f(m, "annual_rate"),
                term_months:        m["term_months"].as_u64().unwrap_or(360) as u32,
                start_date,
                currency,
            })
        } else { None };

        let heloc = if item["heloc"].is_object() {
            let h = &item["heloc"];
            Some(HelocLine {
                credit_line_usd: f(h, "credit_line_usd"),
                draw_rate:       f(h, "draw_rate"),
                ltv_cap:         h["ltv_cap"].as_f64().unwrap_or(0.80),
                enabled:         b(h, "enabled"),
            })
        } else { None };

        let reverse_mortgage = if item["reverse_mortgage"].is_object() {
            let r = &item["reverse_mortgage"];
            Some(ReverseMortgageTerms {
                max_proceeds_local: f(r, "max_proceeds_local"),
                elected:            b(r, "elected"),
            })
        } else { None };

        let rental = if item["rental"].is_object() {
            let r = &item["rental"];
            Some(RentalProfile {
                monthly_rent_jpy:      f(r, "monthly_rent_jpy"),
                monthly_rent_usd:      f(r, "monthly_rent_usd"),
                vacancy_pct:           r["vacancy_pct"].as_f64().unwrap_or(0.05),
                annual_insurance_jpy:  f(r, "annual_insurance_jpy"),
                annual_insurance_usd:  f(r, "annual_insurance_usd"),
                annual_repairs_pct_fmv: r["annual_repairs_pct_fmv"].as_f64().unwrap_or(0.01),
            })
        } else { None };

        Some(RealEstateHolding {
            name:                  s(item, "name"),
            location,
            property_type,
            structure_type,
            purchase_date:         item["purchase_date"].as_str().and_then(parse_date),
            purchase_price_jpy:    f(item, "purchase_price_jpy"),
            purchase_price_usd:    f(item, "purchase_price_usd"),
            current_fmv_jpy:       f(item, "current_fmv_jpy"),
            current_fmv_usd:       f(item, "current_fmv_usd"),
            annual_property_tax_jpy: f(item, "annual_property_tax_jpy"),
            annual_property_tax_usd: f(item, "annual_property_tax_usd"),
            mortgage,
            heloc,
            reverse_mortgage,
            rental,
        })
    }).collect()
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

// ─── V8.1 Detailed expense category helpers ───────────────────────────────────

fn parse_expense_categories(v: &serde_json::Value) -> Vec<crate::models::expense::ExpenseCategory> {
    use crate::models::expense::{ExpenseCategory, CategoryKind};
    let Some(arr) = v.as_array() else { return Vec::new(); };
    arr.iter().filter_map(|item| {
        let obj = item.as_object()?;
        let kind = match obj.get("kind").and_then(|x| x.as_str()).unwrap_or("essential") {
            "discretional" | "discretionary" => CategoryKind::Discretional,
            _ => CategoryKind::Essential,
        };
        Some(ExpenseCategory {
            name:             obj.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string(),
            kind,
            amount_jpy:       obj.get("amount_jpy").and_then(|x| x.as_f64()).unwrap_or(0.0),
            frequency_months: obj.get("frequency_months").and_then(|x| x.as_u64()).map(|n| n.max(1) as u32).unwrap_or(1),
            end_date:         obj.get("end_date").and_then(|x| x.as_str())
                                  .and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()),
            note:             obj.get("note").and_then(|x| x.as_str()).unwrap_or("").to_string(),
        })
    }).collect()
}

fn synthetic_stop_rules_from_categories(
    categories: &[crate::models::expense::ExpenseCategory],
    sim_start: chrono::NaiveDate,
    sim_end: chrono::NaiveDate,
) -> Vec<ExpenseRule> {
    use crate::models::expense::CategoryKind;
    use chrono::Datelike;

    let mut out = Vec::new();
    for cat in categories {
        let Some(end) = cat.end_date else { continue; };
        if end < sim_start || end > sim_end { continue; }

        // Stop rule starts on the first of the month AFTER `end`.
        let next_month = if end.month() == 12 {
            chrono::NaiveDate::from_ymd_opt(end.year() + 1, 1, 1)
        } else {
            chrono::NaiveDate::from_ymd_opt(end.year(), end.month() + 1, 1)
        };
        let Some(start) = next_month else { continue; };
        if start > sim_end { continue; }

        let monthly = cat.effective_monthly_jpy();
        if monthly <= 0.0 { continue; }

        out.push(ExpenseRule {
            // Generic-name (no NHI/Nenkin/ResTax/Education keyword) so engine
            // routes to the base/floor branch in cashflow_engine.rs.
            name: format!("CategoryStop:{}", cat.name),
            amount_jpy: -monthly,
            start_date: start,
            end_date: sim_end,
            apply_to_floor: matches!(cat.kind, CategoryKind::Essential),
            inflate: true,
        });
    }
    out
}
