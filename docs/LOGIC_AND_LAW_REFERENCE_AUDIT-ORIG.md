# Retirement Calculator — Logic, Law-Basis & User-Input Audit

Generated: 2026-05-18
Scope: Stage 12 codebase (v7.11.1, all stages 02–12 enabled by default in code)
Purpose: A single reference document listing every meaningful decision the
application makes, the code that implements it, the typical outputs, the
dependencies/inputs, the legal/procedural rationale (US & Japan), and the
user-facing inputs that change it. Designed to be diffed against current
US and Japanese tax law to surface drift, gaps, or errors.

Format per item:
- **WHAT** — the logic itself
- **WHERE** — file:line / module reference
- **OUTPUT** — typical numeric result and snapshot field
- **DEPENDS ON** — internal data, other engines, prior-year state
- **RATIONALE / LAW** — IRC §, NTA notice, treaty article, or procedure
- **USER INPUT** — config field(s), UI control, why it is an option

Currency convention: `_jpy` = Japanese Yen, `_usd` = US Dollars.

================================================================================
SECTION A — SIMULATION CONTROL FLOW & TIME AXIS
================================================================================

A.1  Monthly tick loop (Jan‑X → Dec‑Y)
- WHAT: The simulation advances one calendar month at a time from
  `cfg.start_date` to `cfg.end_date`. Each month invokes `process_month()`,
  which runs (in this order): new‑year housekeeping → FX drift →
  spouse benefits → retirement transition → Roth rebalance → portfolio
  growth → recession/recovery shocks → salary accumulation → RSU vesting
  → tax‑loss harvesting → dividends → contributions → rebalancing →
  per‑account rebalance strategy → cashflow waterfall → December history
  capture → year‑end tax true‑up → annual snapshot.
- WHERE: `src/simulation/controller.rs:80-348` (`run`, `process_month`)
- OUTPUT: An `AnnualSnapshot` per Dec 31; a `SimResults` at end of horizon.
- DEPENDS ON: `cfg.start_date`, `cfg.end_date`, `cfg.retirement_date`,
  `cfg.rebalance_date`.
- RATIONALE: Engine is calendar‑year deterministic — tax law (IRS Form 1040,
  Japan 確定申告) is calendar‑year scoped; cash flow is monthly because
  Nenkin/NHI/Jido Teate/VA all operate on monthly cadence.
- USER INPUT:
  • `start_date`, `end_date`, `retirement_date`, `rebalance_date` (UI Timing
    section in `input_panel.rs`). Determine horizon and pivot of working→retired
    arithmetic.

A.2  New‑year housekeeping
- WHAT: At month=1 of each year: inflate US tax brackets by `inflation_cola`,
  archive prior‑year social‑insurance paid (NHI + Nenkin) for the resident
  tax deduction, archive prior‑year dividend income for NHI basis, archive
  prior‑year PFIC MTM JPY income for Japan resident tax hand‑off, schedule
  this year's resident‑tax and NHI installments, decay Japan capital‑loss
  carry‑forward, reset annual accumulators, project this year's FERS,
  apply shock events (recession + FX), and grow the Roth IRA contribution
  limit.
- WHERE: `src/simulation/controller.rs:354-408` (`handle_new_year`)
- OUTPUT: `state.tax_engine.rules` mutated (brackets *= 1+CPI), new
  `ExpenseRule`s appended for ResTax Q1–Q4 and 12 NHI installments,
  `state.ira_limit` *= (1 + `cfg.ira_limit_growth`), rounded to $10.
- DEPENDS ON: `cfg.inflation_cola`, `cfg.ira_limit_growth`,
  `state.fers_history`, `state.div_income_history`,
  `state.social_insurance_history`, `state.pfic_mtm_jpy_history`.
- RATIONALE / LAW:
  • US tax bracket inflation: IRC §1(f)(3) (chained CPI‑U; here approximated
    by `inflation_cola`).
  • Resident‑tax installment cadence: Local Tax Act Art. 320(2) — four
    payments due June, August, October, January following.
  • Roth IRA contribution limit annual indexation: IRC §219(b)(5)(C).
- USER INPUT: `inflation_cola`, `inflation_japan`, `ira_limit_growth`.

A.3  FX‑rate trajectory (post‑retirement drift)
- WHAT: When `fx_drift_enabled`, the FX rate (¥/$) walks each month. Two
  modes: cadence‑based (every N months, add `fx_drift_increase_amount_jpy`)
  or continuous (`current_fx *= (1 - rate)^(1/12)`). Both gated to dates ≥
  `retirement_date`.
- WHERE: `src/simulation/controller.rs:165-183`
- OUTPUT: `state.current_fx` evolves; affects every USD↔JPY conversion.
- DEPENDS ON: `cfg.fx_drift_enabled`, `cfg.fx_drift_cadence_months`,
  `cfg.fx_drift_rate`, `cfg.fx_drift_increase_amount_jpy`.
- RATIONALE: Models structural JPY appreciation/depreciation (no published
  legal basis — projection assumption only). Cadence model added in V6.6
  to represent BoJ rate‑adjustment steps.
- USER INPUT: FX Drift checkbox + rate/cadence fields (Input Panel
  Economics section, line 2185).

A.4  Shock events (recession + FX)
- WHAT: For each scheduled event in `recession_events` and `fx_shock_events`,
  apply at the start of the matching calendar year. When BOTH occur in the
  same year, ordering is selectable: `DepreciateThenReprice` (conservative,
  default), `RepriceThenDepreciate`, or `Simultaneous` (snapshot pre‑state,
  commit both together).
- WHERE: `src/simulation/controller.rs:427-559` (`apply_recession_for_year`,
  `apply_fx_shock_for_year`, `apply_year_shocks`)
- OUTPUT: Mutates account asset prices and/or `state.current_fx`; records
  pre/post net‑worth JPY in `AnnualSnapshot.pre_shock_net_worth_jpy` and
  `post_shock_net_worth_jpy`.
- DEPENDS ON: `cfg.recession_events: Vec<RecessionEvent>`,
  `cfg.fx_shock_events: Vec<FXShockEvent>`, `cfg.shock_ordering`.
- RATIONALE: User‑provided stress tests. No statutory basis. Ordering
  matters because JPY‑denominated net worth differs depending on whether
  equity drop is applied before or after currency repricing.
- USER INPUT: Recession schedule, FX shock list, shock ordering combo box
  (UI: 3902–3920).

================================================================================
SECTION B — US FEDERAL & STATE TAX ENGINE
================================================================================

B.1  Standard deduction (US) by filing status
- WHAT: `std_deduction` is one of $35,000 (MFJ default), $14,600 (Single /
  MFS), $21,900 (HoH) in 2024. Inflated each January by `inflation_cola`.
- WHERE: `src/engine/tax/us_tax.rs:355-396` (`TaxRules::for_filing_status`),
  `models/config.rs:483-506` (`TaxRules::default`), `inflate()` at 511-522.
- OUTPUT: Used as the floor below which ordinary income is untaxed.
- DEPENDS ON: `cfg.tax_rules.filing_status`, `cfg.inflation_cola`.
- RATIONALE / LAW: IRC §63(c); 2024 published amounts. MFJ baseline of
  $35,000 in code is the 2026 figure ($29,200 in 2024 official tables grown
  forward in code via `TaxRules::default`).  
  ⚠ AUDIT NOTE — verify the $35,000 default vs current IRS Rev. Proc. for
  the simulation year, and ensure the MFJ default matches the year stamped
  in `TaxRules::default()`.
- USER INPUT: Filing Status dropdown (UI line 2413). Drives the
  `for_filing_status` branch.

B.2  Ordinary‑income tax brackets (US federal)
- WHAT: Stacked progressive marginal brackets. 2024 MFJ:
  10/12/22/24/32/35/37 at $23,200 / $94,300 / $201,050 / $383,900 /
  $487,450 / $731,200 / ∞. Single/MFS: 10/12/22/24/32/35/37 at $11,600 /
  $47,150 / $100,525 / $191,950 / $243,725 / $609,350 / ∞. HoH similarly.
- WHERE: `src/engine/tax/us_tax.rs:355-396`, computation at
  `calc_ordinary_tax_on_stacked` (289-321).
- OUTPUT: Tax on ordinary income; also acts as the bracket for short‑term
  capital gains stacked on top of ordinary.
- DEPENDS ON: `cfg.tax_rules.brackets`, `cfg.tax_rules.std_deduction`.
- RATIONALE / LAW: IRC §1, Tax Cuts and Jobs Act amounts as inflation‑
  adjusted. Sentinel f64::INFINITY at top bracket is required for
  no‑silent‑under‑taxation invariant; fallback synthesised when missing
  (289–321).
- USER INPUT: Filing Status + per‑year inflation; brackets not directly
  user‑editable.

B.3  Long‑term capital gains (US) — 0/15/20% stacked
- WHAT: After standard deduction and ordinary‑income floor, LTCG fills:
  0% bracket up to `ltcg_0_limit` (MFJ $115,000 2026 baseline; 47,025
  Single/MFS), then 15% up to `ltcg_15_limit` (MFJ $700,000; Single
  $518,900), then 20%.
- WHERE: `src/engine/tax/us_tax.rs:120-136`
  (`calculate_liability_with_ftc`)
- OUTPUT: `breakdown["gains_at_0_pct"]`, `gains_at_15_pct`, `gains_at_20_pct`;
  accumulated into `total_tax`.
- DEPENDS ON: `gross_ord` (FERS + RSU + SS + SSDI), `std_deduction`,
  `ltcg_0_limit`, `ltcg_15_limit`.
- RATIONALE / LAW: IRC §1(h) (Net Capital Gain Rates).
- USER INPUT: Filing status (sets all thresholds), `enable_estate_planning`
  (for terminal projection), `tax_jurisdiction` (skips entire engine if
  `JapanOnly`).

B.4  Net Investment Income Tax (NIIT) 3.8%
- WHAT: 3.8% on the lesser of (a) net investment income (here:
  `gross_st_cap + gross_lt_cap`) and (b) excess MAGI over
  `niit_threshold` (MFJ $250,000 default; $200,000 Single/MFS/HoH 2024).
- WHERE: `src/engine/tax/us_tax.rs:139-147`
- OUTPUT: `breakdown["niit_on_gains"]`; added to `federal_tax`.
- DEPENDS ON: `gross_ord + gross_st_cap + gross_lt_cap` as MAGI proxy.
- RATIONALE / LAW: IRC §1411 (Affordable Care Act). NIIT thresholds are
  NOT inflation‑adjusted in the statute — but here they ARE inflated via
  `TaxRules::inflate()` (line 511‑522). ⚠ AUDIT NOTE — divergence from
  statute; verify if intentional (modeling future legislative indexing) or
  a defect.
- USER INPUT: Filing status only; no direct toggle.

B.5  Senior standard deduction add‑on (age ≥ 65)
- WHAT: Adds `senior_addon_per_person` to `std_deduction` for the year if
  user/spouse are ≥ 65 on Dec 31. 2026 MFJ: $1,550/person; Single/HoH:
  $1,950/person.
- WHERE: `src/simulation/controller.rs:877-887` (within `finalize_year_taxes`)
- OUTPUT: Year‑specific boost to std_deduction; restored after that year's
  computation.
- DEPENDS ON: `cfg.birth_date`, `cfg.family_unit.spouse_birth_year`,
  `cfg.tax_rules.senior_addon_per_person`.
- RATIONALE / LAW: IRC §63(f)(1)(A) (additional amount for blind/aged).
- USER INPUT: birth dates (Family Demographics section); no separate toggle.

B.6  Foreign Tax Credit — Japan‑first, §904 basket‑aware
- WHAT: Japan resident tax paid is credited against US federal liability,
  but partitioned into the §904 PASSIVE basket (dividends, interest,
  capital‑gains distributions, PFIC §1296 MTM, capital gains) and the
  GENERAL basket (FERS, SS, taxable SSDI, RSU vest value, §6013(g) spouse
  income). Japan capital‑gains tax (20.315%) is unambiguously passive.
  Japan resident tax is allocated by income proportion. Each basket’s
  credit is capped at its own §904 limit (=`federal_before_ftc × basket_inc
  / total_inc`). Unused credit carries forward per‑basket
  (`ftc_carryover_passive_usd`, `ftc_carryover_general_usd`).
- WHERE: `src/engine/tax/us_tax.rs:41-53` (`compute_ftc_basket_limits`),
  `calculate_liability_with_basket_ftc` (189-233);
  `simulation/controller.rs:892-980`.
- OUTPUT: `breakdown["ftc_passive"]`, `["ftc_general"]`, `["ftc_applied"]`;
  surplus → `state.ftc_carryover_*_usd` → `AnnualSnapshot.ftc_carryover_usd`.
- DEPENDS ON: `state.stats.year_japan_res_tax_jpy`,
  `state.stats.year_japan_cap_gains_tax_jpy`, the income split.
- RATIONALE / LAW: IRC §901 (FTC), IRC §904(d) (basket separation), IRC
  §904(c) (10‑year carryover; here modeled as 1‑year roll, not 10).
  ⚠ AUDIT NOTE — §904(c) statutory carryover is 10 years (and 1 year carry‑back).
  Code only rolls one year and never expires. Verify expected behaviour.
- USER INPUT: `us_tax_strategy` (FtcOnly vs FeieAndFtc), `tax_jurisdiction`
  (Both required to engage FTC).

B.7  Foreign Earned Income Exclusion (FEIE)
- WHAT: When `us_tax_strategy = FeieAndFtc`, exclude up to FEIE_LIMIT_2026
  ($126,500 for 2026) from EARNED income only (RSU vest value); FTC then
  applied with `(total_jp_taxable − feie_exclusion) / total_jp_taxable`
  ratio to prevent double‑credit on excluded income.
- WHERE: `src/engine/tax/us_tax.rs:248-280`
  (`calculate_liability_with_feie_ftc`), FEIE_LIMIT_2026 at line 5.
- OUTPUT: `result.feie_exclusion`, `result.feie_applied`; reduced
  `total_tax`.
- DEPENDS ON: `cfg.us_tax_strategy`, RSU vest value (year_rsu_vest_usd).
- RATIONALE / LAW: IRC §911 (FEIE); §911(d)(6) anti‑double‑dip
  proportioning of FTC; annual limit per Rev. Proc. ($126,500 for 2026).
- USER INPUT: `us_tax_strategy` dropdown (UI 2386‑2393).

B.8  SSDI Combined Income Rule (taxable share of SS/SSDI)
- WHAT: provisional_income = AGI_before_SSDI + 0.5 × annual_SSDI. Brackets:
  ≤ $32K → 0% taxable; $32K – $44K → min(0.5×(PI−32K), 0.5×SSDI); > $44K
  → min(85%×SSDI, $6,000 + 0.85×(PI−44K)). Thresholds are MFJ values.
- WHERE: `src/engine/tax/us_tax.rs:335-348`
  (`ssdi_combined_income_taxable_portion`); engaged at
  `controller.rs:817-823`.
- OUTPUT: `ssdi_taxable` USD figure summed into `general_ord` for §904
  basket routing.
- DEPENDS ON: `state.stats.year_ssdi_gross_usd`, base ordinary income.
- RATIONALE / LAW: IRC §86 (Combined Income rule applies identically to SS
  retirement and SSDI). Thresholds are $32K/$44K (MFJ) and $25K/$34K
  (Single). ⚠ AUDIT NOTE — code hard‑codes MFJ thresholds; if the user
  filing status is Single/MFS/HoH the same MFJ numbers are used (defect or
  by design?).
- USER INPUT: `ssdi_monthly_usd`, `ss_monthly_usd`, filing status.

B.9  Per‑state income tax rate
- WHAT: Flat or representative marginal rate by 2‑letter postal code.
  Zero‑tax states (AK/FL/NV/NH/SD/TN/TX/WA/WY) return 0. Graduated‑rate
  states return the rate for ~$50K–$100K AGI (e.g. CA 9.3%, NY 6.85%,
  OR 9.9%).
- WHERE: `src/engine/tax/us_tax.rs:407-456` (`state_tax_rate`)
- OUTPUT: Multiplier on `(gross_ord + total_gains − std_deduction)`,
  added to `total_tax` and (separately) tracked as `state_tax` for FTC
  isolation.
- DEPENDS ON: `cfg.tax_rules.us_state_code`, `cfg.tax_rules.us_state_rate`.
- RATIONALE / LAW: Approximate 2024 state‑tax tables. NOT credited by Japan
  FTC. State estate tax NOT modeled (varies).
- USER INPUT: State postal code dropdown (UI 2423).

B.10  Per‑account §5.1 tax‑routing gate (dividends)
- WHAT: Dividends route US/Japan tax based on the account’s
  `AccountJurisdiction` (Us / Japan / Both / None) and
  `us_tax_advantaged` / `japan_tax_advantaged` flags. Roth = Both →
  US tax‑adv = true → no US tax; iDeCo/NISA = Japan‑adv = true → no Japan
  tax. Taxable = Both → both taxes apply.
- WHERE: `src/handlers/dividends.rs:128-144, 297-377`
- OUTPUT: `state.current_month_div_net_usd`, `_jpy`, plus
  `year_div_tax`, `year_japan_cap_gains_tax_jpy`.
- DEPENDS ON: Each account's enum flags loaded from JSON.
- RATIONALE / LAW: Treaty Article 18 (pensions), domestic IRC §408A (Roth),
  Japan FSA NISA Act, iDeCo (Confirmed‑Contribution Pension Act). Tax‑
  free shielding is a regulatory feature, not a treaty bypass.
- USER INPUT: Account jurisdiction & tax‑advantaged flags (UI 2521‑2608).

================================================================================
SECTION C — JAPAN TAX ENGINE
================================================================================

C.1  Employment income deduction (給与所得控除)
- WHAT: NTA 2024 table. ≤¥550K → all; ≤¥1.625M → ¥550K flat; ≤¥1.8M →
  40% − ¥100K; ≤¥3.6M → 30% + ¥80K; ≤¥6.6M → 20% + ¥440K; ≤¥8.5M →
  10% + ¥1.1M; > ¥8.5M → cap ¥1.95M.
- WHERE: `src/engine/tax/japan_tax.rs:21-37` (`employment_deduction`)
- OUTPUT: Reduces `gross_salary` for both resident‑tax and income‑tax
  bases.
- DEPENDS ON: gross salary (state.stats.year_salary_jpy plus RSU
  vest_jpy).
- RATIONALE / LAW: 所得税法 第28条; 公示 NTA Income Tax Schedule. ⚠ AUDIT
  NOTE — Verify 2024 table values; the law published a small revision
  effective FY2020+ (¥550K floor). Code matches FY2020+ schedule.
- USER INPUT: indirectly via Japan salary fields (`total_annual
  _compensation_usd`, working‑year accumulators).

C.2  Public pension deduction (公的年金等控除)
- WHAT: Age‑split table. Under 65: full deduction up to ¥600K, flat ¥600K
  up to ¥1.3M; 65+ : full up to ¥1.1M, flat ¥1.1M up to ¥3.3M; then
  25%+¥275K, 15%+¥685K, 5%+¥1.455M tiers.
- WHERE: `src/engine/tax/japan_tax.rs:42-60` (`pension_deduction`)
- OUTPUT: Reduces FERS+SS+SSDI+Nenkin gross before resident tax.
- DEPENDS ON: gross pension JPY, age (yr − birth_date.year()).
- RATIONALE / LAW: 所得税法 第35条; NTA tables. SS and FERS routed as
  公的年金等 per US‑Japan Treaty Art. 17.
- USER INPUT: birth date, all pension fields, jurisdiction flags.

C.3  Japan resident tax (住民税)
- WHAT: `(net_salary + net_pension − basic_ded − spouse_dep_dep − soc_ins)
  rounded ↓ to ¥1K × income_rate + per_capita_jpy`. Basic deduction
  ¥430K; spouse/dep deduction ¥330K each (≤¥9M), ¥220K (≤¥9.5M), ¥110K
  (≤¥10M), 0 above. Per‑capita: ¥6,000 standard, Nagoya 9.7% income +
  ¥6,000 per‑capita.
- WHERE: `src/engine/tax/japan_tax.rs:74-109` (`calculate_resident_tax`),
  rates lookup in `japan_regions.rs`.
- OUTPUT: Annual tax bill, split into 4 quarterly ExpenseRules (June,
  August, October, January).
- DEPENDS ON: `cfg.prefecture`, `cfg.city`, prior‑year salary+RSU+pension+
  PFIC MTM, social insurance history (NHI + Nenkin paid).
- RATIONALE / LAW: 地方税法 第32条 (incomes), 第38条 (basic), 第34条
  (spouse), 第37条 (dependents). Nagoya special rate via municipal
  ordinance 名古屋市市民税減税条例. Forest‑environment ¥1,000 component
  in per‑capita per Reconstruction & Forest Env. Tax Act 2024+.
  ⚠ AUDIT NOTE — Per‑capita includes Forest Environment Tax (¥1,000)
  starting 2024 — confirmed in `japan_regions.rs:3-4` comment.
- USER INPUT: Prefecture / City dropdowns (UI 2456‑2469).

C.4  Japan income tax (所得税) — working years only
- WHAT: Progressive brackets on (net_salary + net_pension − basic_dec
  ¥480K − spouse_dep ¥380K × deps − soc_ins): 5/10/20/23/33/40/45 %
  at ¥1.95M/¥3.3M/¥6.95M/¥9M/¥18M/¥40M, then × 1.021 reconstruction
  surcharge.
- WHERE: `src/engine/tax/japan_tax.rs:121-165`
  (`calculate_income_tax`); fired at `controller.rs:772-799`
  (December of pre‑retirement years).
- OUTPUT: `state.stats.year_japan_income_tax_jpy` →
  `AnnualSnapshot.japan_income_tax_jpy`.
- DEPENDS ON: `state.stats.year_salary_jpy + year_rsu_vest_jpy`,
  social insurance, age, dependents.
- RATIONALE / LAW: 所得税法 第89条 (brackets); 復興特別所得税法 1.021×
  surcharge through 2037.
- USER INPUT: salary fields; dependents.

C.5  Japan capital‑gains tax — Tokyo Stock Exchange standard
- WHAT: Flat 20.315 % (15.315 % 国税 + 5 % 住民税) applied on realised
  gain (jpy_proceeds − jpy_basis) at sale or dividend distribution.
- WHERE: `src/handlers/cashflow_manager.rs:13` (`JAPAN_CAPITAL_GAINS_RATE`),
  used in `v7_liquidate_for_deficit` (817-973), `dividends.rs:305-310`,
  rebalancing handlers.
- OUTPUT: `year_japan_cap_gains_tax_jpy`, eventually credited via §904
  passive FTC and surfaced in `AnnualSnapshot.japan_cap_gains_tax_jpy`.
- DEPENDS ON: per‑share JPY basis (avg_purchase_price_jpy or
  avg_cost × FX_at_load).
- RATIONALE / LAW: 租税特別措置法 第37条の11 (上場株式等の譲渡所得課税).
  20% + 0.315% (復興特別所得税 = 0.315% = 15% × 0.021).
- USER INPUT: positions' `avg_purchase_price_jpy` field, withdrawal
  strategy.

C.6  Japan capital‑loss carry‑forward
- WHAT: When a sale realises a loss, the JPY‑denominated loss is added to
  `state.japan_loss_carryforward_jpy`. Each January the new‑year
  housekeeping rolls accumulated losses into the carry‑forward; the engine
  treats it as a single rolling balance.
- WHERE: `src/simulation/controller.rs:377-384`, signed‑gain check in
  `cashflow_manager.rs:911-927` and TLH path
  `tax_loss_harvesting.rs:97-110`.
- OUTPUT: `state.japan_loss_carryforward_jpy`.
- RATIONALE / LAW: 租税特別措置法 第37条の12の2 — 3‑year carry‑forward
  of listed‑security losses.
- ⚠ AUDIT NOTE — Statutory 3‑year cap is NOT enforced; carryover never
  expires. Code comment notes “implicit expiry” via offset against gains,
  but a long string of loss years would over‑shelter future income.
- USER INPUT: `tlh_enabled`, `tlh_active_months`, `tlh_min_loss_usd`.

C.7  Japan crypto miscellaneous income (雑所得)
- WHAT: When `cfg.crypto_tax_enabled = true` AND `asset.is_crypto()`,
  Japan tax on the realised crypto gain uses the taxpayer's marginal
  ordinary‑income rate (`estimate_marginal_rate`: national 1.021 ×
  bracket + 10% resident, capping at ~55%) instead of the flat 20.315%
  cap‑gains rate.
- WHERE: `src/engine/tax/japan_tax.rs:187-226`
  (`miscellaneous_income_tax_jpy`, `estimate_marginal_rate`); applied at
  `cashflow_manager.rs:870-927`.
- OUTPUT: Higher `year_japan_cap_gains_tax_jpy` on crypto sales.
- RATIONALE / LAW: NTA 仮想通貨に関する所得の計算等 (2017‑12‑01 公示);
  cryptocurrencies are 雑所得 (Miscellaneous Income) under 所得税法 第35
  条 — taxed at progressive marginal rates.
- USER INPUT: `crypto_tax_enabled` (default true), asset class set to
  `Crypto` per position.

C.8  Reconstruction surcharge 復興特別所得税 (1.021×)
- WHAT: Multiplier 1.021 on national income tax. Cap‑gains: incorporated
  in the 20.315% headline rate (15% × 1.021 ≈ 15.315%).
- WHERE: `japan_tax.rs:163-164` (`calculate_income_tax`).
- RATIONALE / LAW: 復興特別所得税法 — Special Income Tax for
  Reconstruction (effective 2013‑01‑01 through FY2037).
- ⚠ AUDIT NOTE — Sunset of surcharge after FY2037 not modeled.

C.9  Exit tax (出国税) monitor — IT Act Art. 60‑2
- WHAT: Annual check at Dec 31. If `cfg.japan_residency_start_date` is set
  AND user has lived in Japan ≥ 5 of last 10 years, AND the total of
  qualifying assets ≥ ¥100M, emit `exit_tax_triggered = true`.
- WHERE: `src/simulation/controller.rs:1136-1158`
  (`evaluate_exit_tax_trigger`)
- OUTPUT: `AnnualSnapshot.exit_tax_triggered`,
  `exit_tax_asset_value_jpy`.
- DEPENDS ON: `cfg.japan_residency_start_date`,
  `cfg.exit_tax_include_tax_advantaged`.
- RATIONALE / LAW: 所得税法 第60条の2 (国外転出時課税). Triggers
  deemed‑sale of equity assets > ¥100M.
- USER INPUT: Japan residency start date; include‑advantaged toggle
  (NISA/iDeCo).

================================================================================
SECTION D — NATIONAL HEALTH INSURANCE (国民健康保険 / NHI)
================================================================================

D.1  NHI premium engine — Calculated mode
- WHAT: Per‑municipality formula. For each component (Medical 医療分,
  Support 支援分, Nursing 介護分):
  `component = min(income_basis × rate + per_capita × n, cap)`. Income
  basis = `max(0, net_salary + net_pension + invest_inc − ¥430K)`. Nursing
  applies only for ages 40–64. Defaults are Sagamihara City 2026 rates
  (8.46% / 2.04% / 2.02% income rates; per‑capita ¥33,600 / ¥11,400 /
  ¥12,600; caps ¥650K / ¥240K / ¥170K).
- WHERE: `src/engine/tax/nhi.rs:77-122` (`compute_from_rates`); rate table
  defaults `models/config.rs:104-117`.
- OUTPUT: Annual JPY premium, split into 12 monthly ExpenseRules.
  Tracked in `state.stats.year_exp_nhi`.
- DEPENDS ON: prior‑year salary, pension, investment income (gated by
  `include_us_investment_income`), number insured (incl. fractional
  per Stage 03 monthly precision), age.
- RATIONALE / LAW: 国民健康保険法 + 各市町村の保険料賦課条例.
  Sagamihara‑specific rates published annually.
- ⚠ AUDIT NOTE — Default rates are 2026‑projected; verify actual
  municipality publishes 2026 schedule with these exact values.
- USER INPUT: NHI mode selector — Calculated vs ManualOverride vs
  NinkiKeizoku; per‑rate UI overrides at 2244‑2294;
  `include_us_investment_income` toggle.

D.2  NHI Manual Override mode
- WHAT: User supplies two static annual totals: `spike_year_total_jpy`
  (used in first post‑retirement calendar year) and
  `ongoing_annual_total_jpy` (every year thereafter).
- WHERE: `nhi.rs:40-43`; UI inputs 2307‑2317.
- OUTPUT: Same — populates monthly ExpenseRules.
- RATIONALE: Lets users with a real NHI invoice override the model.
- USER INPUT: Two JPY fields under NHI Manual Override.

D.3  NHI 任意継続 (Ninki Keizoku) Voluntary Continuation
- WHAT: For ≤ 24 months, keep the employer Shakai Hoken at a fixed
  `monthly_premium_jpy`; afterward, fall back to the wrapped NhiModel.
- WHERE: `nhi.rs:46-61`; tracked via
  `state.nhi_ninki_keizoku_months_remaining` at `controller.rs:692-719`.
- OUTPUT: Monthly NHI ExpenseRule populated at the fixed amount during
  the window.
- RATIONALE / LAW: 健康保険法 第37条 (HIA Art. 37) — voluntary
  continuation of employee insurance up to 24 months at full premium.
- USER INPUT: Selecting NinkiKeizoku model, monthly amount, duration,
  fallback model.

D.4  NHI 1‑year income lookback / spike year
- WHAT: NHI assessed in June against prior calendar year's income
  (`prev_year_gross_salary_jpy`). The first post‑retirement year still
  reflects peak working income → the “spike year” (~¥767K typical).
- WHERE: Scheduling at `controller.rs:657-766`
  (`schedule_annual_nhi`); spike conditional `is_spike_year =
  prev_year == retirement_year`.
- OUTPUT: First post‑retirement year ExpenseRule has the spike level.
- RATIONALE / LAW: 国民健康保険法施行令 — 保険料賦課年度の対象所得は
  前年中の所得. Engineered into the simulator to surface bridge‑fund
  sizing needs.
- USER INPUT: retirement_date; `retirement_year_gross_income_jpy`.

D.5  Fractional `num_insured` (Stage 03 monthly precision)
- WHAT: When `cfg.monthly_dependent_precision = true`, per‑capita NHI
  charges scale by `num_insured = 1 + sum(months_under_18_in_year(dep)/12)`.
  Default false (NTA‑style Dec‑31 snapshot, integer count).
- WHERE: `controller.rs:721-740`.
- OUTPUT: A child turning 18 mid‑year no longer doubles the per‑capita
  for the whole year.
- RATIONALE: NTA per‑capita is technically annual; this is an engine
  refinement for budget realism, not a statutory requirement.
- USER INPUT: `monthly_dependent_precision` checkbox (UI 3448).

================================================================================
SECTION E — LONG‑TERM CARE INSURANCE (介護保険 / Kaigo Hoken)
================================================================================

E.1  Ages 40–64: bundled in NHI
- WHAT: 第2号被保険者 premium folded into NHI’s 介護分 component
  (covered in D.1).

E.2  Ages 65+: separate 9‑tier bracket premium
- WHAT: `calculate_age_65_plus_premium_annual(annual_pension_jpy,
  brackets)` walks the user’s municipal bracket table, returning the
  first matching annual premium. Default = Sagamihara 2026:
  ¥30K (≤¥800K) → ¥45K → ¥60K → ¥75K → ¥85K → ¥95K → ¥110K → ¥130K
  → ¥150K (>¥5M).
- WHERE: `src/engine/tax/kaigo_hoken.rs:138-151`; bracket table at
  76-110; monthly expense computed in `cashflow_engine.rs:151-206`.
- OUTPUT: Monthly ExpenseRule (`exp.kaigo_hoken`) → `year_kaigo_premium_jpy`.
- DEPENDS ON: age, sum of Nenkin + FERS + SS (converted to JPY) as
  bracket lookup.
- RATIONALE / LAW: 介護保険法 + municipal 介護保険料条例. Sagamihara FY2026
  notice https://www.city.sagamihara.kanagawa.jp/kurashi/kenko/1026531/1007427.html
- ⚠ AUDIT NOTE — Default brackets are “illustrative” per code comment
  (line 74); users should verify against actual municipal notice.
- USER INPUT: `kaigo_hoken_enabled` toggle, optional custom brackets,
  prefecture choice.

E.3  Optional out‑of‑pocket care projection
- WHAT: `projected_out_of_pocket_care(age, scenario)`. Scenarios:
  None → ¥0; Low → ¥20K/mo from age 75; Medium → ¥40K/mo from age 75;
  High → ¥80K/mo from age 80.
- WHERE: `kaigo_hoken.rs:173-186`; engaged in `cashflow_engine.rs:192-198`.
- OUTPUT: Added to monthly expense; tracked separately as
  `year_kaigo_care_jpy`.
- RATIONALE: NOT statutory — engine projection of average out‑of‑pocket
  care after insurance coverage. Stress‑test aid only.
- USER INPUT: `kaigo_care_scenario` dropdown (None / Low / Medium / High).

================================================================================
SECTION F — JAPAN NENKIN, JIDO TEATE, VA, FERS, SOCIAL SECURITY
================================================================================

F.1  Japanese Nenkin pension income
- WHAT: Constant monthly amount `nenkin_income_monthly_jpy` starting in
  the year `birth.year + nenkin_income_start_age`, inflated by
  `inflation_japan` thereafter.
- WHERE: `cashflow_engine.rs:324-335`.
- OUTPUT: `IncomeBreakdown.nenkin_income_jpy`; routes to Tier 0 of the
  defensive waterfall.
- RATIONALE / LAW: 国民年金法 / 厚生年金保険法 — payable from age 65
  (deferral / acceleration permitted 60–75). Simplified to a single
  start age with no de‑facto deferral premium.
- USER INPUT: Monthly amount, start age (UI 3681‑3711).

F.2  Jido Teate (児童手当) child allowance — Tier 0.5
- WHAT: Bi‑monthly. ¥15K/month ages 0–<3; ¥10K/month ages 3–<18. Paid
  in even calendar months as 2× the per‑month rate. V7.4 fixed
  age‑boundary drift by accruing per covered month not per payment.
- WHERE: `cashflow_manager.rs:1018-1060` (`compute_jido_teate_jpy`,
  `jido_teate_for`, `monthly_jido_rate`).
- OUTPUT: `state.stats.year_jido_teate_jpy`; added to Tier 0 JPY floor.
- DEPENDS ON: `cfg.child_birth_date`, `cfg.jido_teate_enabled`.
- RATIONALE / LAW: 児童手当法 (Child Allowance Act); 2024 amounts and
  abolition of income cap effective 2024‑10‑01.
- ⚠ AUDIT NOTE — Income cap explicitly NOT modeled (per code comment).
  Verify the post‑Oct‑2024 no‑cap rule still applies in target year.
- USER INPUT: Toggle + child birth date (Family Demographics).

F.3  VA disability compensation (2026 monthly rates)
- WHAT: Official 2026 monthly USD by rating (10/20/.../100%) and
  dependent status (VetOnly / WithSpouse / WithSpouseAndChild). Inflated
  by COLA from 2026. Plus optional SMC variants (K, L, L½, M, M½, N, N½,
  O/P, R.1, R.2, Housebound) and `va_monthly_override` / `smc_monthly_override`.
- WHERE: `src/engine/va_benefits.rs:23-42` (`lookup_va_monthly_2026`),
  61-90 (`lookup_smc_monthly_2026`); income wiring at
  `cashflow_engine.rs:236-301`.
- OUTPUT: `IncomeBreakdown.va_usd` → Tier 4 USD floor.
- DEPENDS ON: rating, dependent status, `va_child_cutoff_date`, college‑
  student flag (extends to age 23 under 38 CFR §3.57), SMC variant.
- RATIONALE / LAW: 38 USC §1114 disability rates; published by VA effective
  Dec 1 each year; tax‑free per IRC §104(a)(4) and US‑Japan Treaty Art.
  19. SMC per 38 USC §1114(k)–(s).
- USER INPUT: Rating, dependent status, SMC variant, manual overrides
  (UI 3479‑3578).

F.4  Social Security retirement (age 67 default)
- WHAT: Constant USD `ss_monthly_usd` starting year `birth.year +
  ss_start_age`, inflated by COLA. Counted as ordinary income for US;
  routed through 公的年金等控除 in Japan per Treaty Art. 17.
- WHERE: `cashflow_engine.rs:310-321`; US tax incorporates at
  `controller.rs:812`.
- OUTPUT: `IncomeBreakdown.ss_usd`; SS combined‑income rule applies
  (B.8).
- RATIONALE / LAW: SSA: 42 USC §402; US‑Japan Totalization Agreement
  (Treaty Art. 17). Default age 67 = full retirement age for cohorts
  born ≥ 1960.

F.5  SSDI → SS reclassification at age 65
- WHAT: SSDI dollar amount inflates with COLA from 2026; classification
  switches to SS retirement at age 65 (amount unchanged), but no
  separate handling needed since both flow as ordinary income.
- WHERE: `cashflow_engine.rs:340-346`.
- RATIONALE / LAW: 42 USC §423(a)(1) — SSDI converts to retirement at full
  retirement age (here simplified to 65). Tax treatment unchanged.

F.6  FERS pension with diet‑COLA
- WHAT: `fers_monthly_start * (1 + diet_cola)^years_compounding`. Diet
  COLA: CPI ≤ 2% → full CPI; ≤ 3% → capped at 2%; > 3% → CPI − 1%. COLA
  begins year after the retiree turns 62.
- WHERE: `cashflow_engine.rs:77-93`, diet rule 61-70.
- OUTPUT: `IncomeBreakdown.fers_usd`.
- RATIONALE / LAW: 5 USC §8462 — Cost‑of‑Living Adjustment for FERS
  Annuitants (the “diet” formula).
- USER INPUT: `fers_monthly_start`, `fers_start_date`,
  `fers_jurisdiction` (Both / UsOnly / JapanOnly / TaxFree),
  `fers_japan_local_tax_exempt` (Treaty Art. 18 toggle).

F.7  US‑Japan Treaty Article 18 (government pensions)
- WHAT: When `cfg.fers_japan_local_tax_exempt = true` OR
  `cfg.fers_jurisdiction = UsOnly`, FERS gross is excluded from the
  Japan resident‑tax base.
- WHERE: `controller.rs:590-594`.
- RATIONALE / LAW: US‑Japan Tax Treaty Article 18(2) — pensions and other
  similar remuneration paid by a Contracting State to a national thereof
  shall be taxable only in that State.
- USER INPUT: `fers_japan_local_tax_exempt` toggle (UI 3595‑3596).

F.8  US‑Japan Treaty Article 17 (private/SS pensions)
- WHAT: Modeled implicitly by routing SS through Japan public‑pension
  deduction (公的年金等控除) and into the resident‑tax base.
- WHERE: `controller.rs:595-596, 600`.
- RATIONALE / LAW: US‑Japan Tax Treaty Article 17 — pensions arising in a
  Contracting State and paid to a resident of the other Contracting
  State shall be taxable only in that other State, subject to the
  savings clause (Art. 1(5)).

F.9  Military retired pay
- WHAT: Optional `MilitaryRetiredConfig { monthly_usd, jurisdiction }`. If
  jurisdiction != TaxFree, the amount is added to Tier 4 USD floor.
- WHERE: `cashflow_manager.rs:176-180`.
- RATIONALE / LAW: Military Retired Pay is taxable in both US and Japan
  per Treaty Art. 18 savings clause; the user can override jurisdiction
  if domestic statute or treaty position differs.

================================================================================
SECTION G — DISTRIBUTION ROUTING & COMPONENT‑TYPED DIVIDENDS (V7.6)
================================================================================

G.1  Component decomposition of distributions
- WHAT: Per‑asset `DetailedReturnProfile { cap_growth, nav_growth,
  dividend_yield, interest_yield, cap_gains_dist, special_dist, roc,
  expense_ratio }`. Engine separately emits up to 5 typed events per
  dividend month per asset.
- WHERE: `src/handlers/dividends.rs:49-88` (`collect_distribution_events`).
- OUTPUT: Distinct stat buckets: `year_dist_dividend_usd`,
  `year_dist_interest_usd`, `year_dist_cap_gains_usd`, `year_dist_special_usd`,
  `year_dist_roc_usd`; aggregate `year_div_gross`, `year_cap_gains`.
- RATIONALE: Drives §904 basket routing (Interest → passive ord;
  Special → passive ord; CapGainsDist → passive ord if PFIC §1296 or
  LTCG bucket otherwise). ROC (§301(c)(2)) is non‑taxable; reduces basis
  via `apply_roc_basis_reduction` and routes any excess above basis to
  cap‑gains.
- USER INPUT: Position‑level “Use detailed return profile” toggle (UI 3037).

G.2  ROC basis reduction & excess
- WHAT: `Asset.apply_roc_basis_reduction(amount_usd, fx)` reduces FIFO‑lot
  basis. Any amount in excess of remaining basis is returned and routed
  to `year_cap_gains` (passive LTCG) and (Japan side) hit with
  20.315%.
- WHERE: `dividends.rs:148-213`.
- RATIONALE / LAW: IRC §301(c)(2) — ROC reduces basis dollar‑for‑dollar;
  §301(c)(3) — once basis exhausted, treated as capital gain.

G.3  PFIC §1296 mark‑to‑market on Japan‑domiciled funds
- WHAT: Annual gain = (current_price − pfic_prior_year_fmv) × qty,
  banked in USD AND JPY. Losses → §1296(d) carry‑forward (USD), reduces
  reportable gain in subsequent years. Drift between USD×FX and stored
  JPY basis triggers a `PficDriftWarning` and self‑heals when drift > 1%
  (and `track_pfic_basis_drift = true`).
- WHERE: `src/engine/tax/pfic.rs:26-155`; called by
  `controller.rs:824-835`.
- OUTPUT: `year_pfic_mtm_income_usd` (passive ord basket),
  `year_pfic_mtm_income_jpy` (Japan resident tax on non‑NISA/iDeCo).
- DEPENDS ON: Asset `pfic_regime = Mtm`, `japan_tax_advantaged` flag.
- RATIONALE / LAW: IRC §1296 — Election of Mark‑to‑Market for PFIC stock
  (passive basket per §904(d)(1)(B)). Loss carry‑forward §1296(d).
  Japan side: PFIC MTM is phantom US income — Japan does not recognise
  unrealised gains, so the FTC pool gains nothing from Japan tax —
  un‑hedged drag.
- USER INPUT: Per‑asset `pfic_regime` selector (NotPfic / Mtm / Qef /
  ExcessDistribution), `track_pfic_basis_drift` toggle (UI 2401).

================================================================================
SECTION H — CASHFLOW WATERFALL (DEFENSIVE V7.1)
================================================================================

The post‑retirement monthly distribution waterfall, in order:

H.1  Tier 0 — JPY Floor Income (Nenkin + DC payout + Jido Teate + Rental JPY)
- WHAT: Pure JPY sources; no FX conversion.
- WHERE: `cashflow_manager.rs:232-237`.

H.2  Tier 0.5 — Jido Teate (covered in F.2).

H.3  Tier 1 — JPY Dividends
- WHAT: Net JPY‑denominated dividends paid this month (`current_month_div_net_jpy`).
- WHERE: `cashflow_manager.rs:240-242`.

H.4  Tier 2 — reserved.

H.5  Tier 2.5 — Education Fund (bypass)
- WHAT: Tagged Education expenses (ExpenseRule name contains
  "Education") draw from `state.education_fund_jpy` first; residual
  falls through to a Tier‑8 sale sized to the shortfall. Fund
  accumulates monthly skim from JPY surplus (capped at
  `cfg.edu_savings_jpy_monthly`).
- WHERE: `cashflow_manager.rs:209-211, 1103-1138`.
- USER INPUT: `enable_education_savings` toggle,
  `edu_savings_jpy_monthly` field.

H.6  Tier 3 — JPY War Chest draw.

H.7  Tier 4 — USD Floor Income (FERS + VA + SS + SSDI + Mil + Rental USD)
  converted to JPY with `fx_spread_penalty` (default 0.5%).

H.8  Tier 5 — USD Dividends this month → JPY w/ penalty.

H.9  Tier 6 — USD Bridge Fund → JPY w/ penalty. Bridge depletion flagged
  as `bridge_exhausted = true`.

H.10  Tier 7 — Belt‑tighten / Target drop to Minimum
- WHAT: Shielded regime: if gap remains OR both buffers are zero, drop
  the month's spend target from Base (`base_expense_jpy * inflation`) to
  Minimum (`min_expense_jpy * inflation`); reduces `gap` by the savings
  (capped at remaining gap). Logs `T7-A`. Dynamic regime is preemptive
  (sells in advance to restock buffers).
- WHERE: `cashflow_manager.rs:321-441`.

H.11  Tier 7.5 — HELOC draw
- WHAT: When `enable_heloc_tier` true and a property has an active HELOC,
  draws USD against `(ltv_cap × fmv) − mortgage_balance`, capped by
  remaining credit line. Converted to JPY with FX penalty.
- WHERE: `cashflow_manager.rs:1150-1180`; `real_estate_engine.rs:104-114`.
- RATIONALE / LAW: HELOC is a draw against home equity (US: Tax Cuts &
  Jobs Act §11043 limits interest deduction for non‑acquisition‑debt
  HELOC; Japan side has no parallel facility, so a JP property HELOC is
  rare in practice).
- USER INPUT: per‑property HelocLine, `enable_heloc_tier` toggle.

H.12  Tier 8 — Highest‑JPY‑basis‑first liquidation
- WHAT: Sells Taxable holdings in order of highest `jpy_basis_per_share`.
  Per share, gross‑up withholds Japan 20.315% on jpy_gain (or marginal
  rate for crypto) and US state cap‑gains. Roth fallback as last resort
  (tax‑free).
- WHERE: `cashflow_manager.rs:817-973` (`v7_liquidate_for_deficit`).
- OUTPUT: `year_cap_gains`, `year_forced_liquidations_usd`,
  `year_japan_cap_gains_tax_jpy`, `year_state_cap_gains_tax_usd`.
- RATIONALE: Highest JPY‑basis first minimises realised JPY gain (fewer
  yen of capital gain → less 20.315%), preserving portfolio longevity.

H.13  Tier 9 — Estate Planning Gift Sink (December)
- WHAT: In December, when `enable_gift_sink = true`, drain up to
  `annual_gift_jpy_per_recipient × gift_recipient_count` from the JPY
  surplus into `gift_sink_jpy`. Per‑recipient check vs
  `us_gift_exclusion_usd` flags `year_form_709_required`.
- WHERE: `cashflow_manager.rs:1381-1396`.
- RATIONALE / LAW: 暦年贈与 (Japan annual gifting under 相続税法 第21
  条の5; ¥1.1M/recipient exempt). US side: IRC §2503(b) annual
  exclusion ($19,000 per donee for 2026); IRS Form 709 required if
  exceeded.
- USER INPUT: `enable_gift_sink`, `annual_gift_jpy_per_recipient`,
  `gift_recipient_count`, `us_gift_exclusion_usd`.

H.14  Cautious waterfall (V7.0 legacy)
- WHAT: Alternative `manage_monthly_cashflow_cautious`. Cuts spending to
  available income first; buffers are a last resort.
- WHERE: `cashflow_manager.rs:495-631`.
- USER INPUT: `withdrawal_waterfall = Cautious`.

================================================================================
SECTION I — RSU AWARDS & TRANSITION
================================================================================

I.1  RSU vesting schedule generation
- WHAT: Per award: `vesting_years`, `vesting_months`, optional
  `vesting_months_total`, `vesting_start_date` (overrides grant_date),
  `cliff_vest_months` (accumulates skipped events into the first
  post‑cliff event). Events after `retirement_date` are forfeited.
- WHERE: `src/engine/rsu_engine.rs:29-117`.
- OUTPUT: `vesting_schedule: Vec<VestingEvent>`.
- USER INPUT: RSU table (UI 4002‑4237) — ticker, grant date, units,
  cadence, cliff, etc.

I.2  RSU vesting tax — SALARY mode
- WHAT: Marginal US tax computed by stacking vest value on current
  ordinary; recorded as externally paid; full vest value buys into
  Taxable.
- WHERE: `handlers/rsu_vesting.rs:166-174`.
- RATIONALE / LAW: IRC §83 — RSUs taxable as ordinary at vest; W‑2
  withholding standard practice.

I.3  RSU vesting tax — SELL_TO_COVER with realism
- WHAT: When `rsu_sell_to_cover_realism = true`, computes combined US +
  Japan marginal tax (income + resident) on the vest; if vest value ≥
  combined bill, buys net; if not (recession‑driven margin call),
  drains Bridge → War Chest → Tier‑8 sale; records residual as
  `state.unpaid_rsu_tax_liability_usd` and emits an
  `RsuSellToCoverWarning`.
- WHERE: `handlers/rsu_vesting.rs:71-198`.
- RATIONALE / LAW: §83 mark‑to‑market combined with the user’s 2024 NTA
  income‑tax + 住民税 marginal stack.
- USER INPUT: `rsu_tax_handling` ("SALARY" / "SELL_TO_COVER"),
  `rsu_sell_to_cover_realism`, `rsu_sell_to_cover_policy`
  (Strict/Permissive).

I.4  Retirement transition (one‑time portfolio rebalance)
- WHAT: At `cfg.rebalance_date`:
  1. Optional recession shock.
  2. Compute cash targets: war chest + bridge fund + Japan resident
     tax (`estimate_resident_tax_transition`) + US cap‑gains tax on
     realised gains.
  3. ≤ 15 iterations of proportional sales from Taxable (each
     sized to the current shortfall), updating realised gains and
     recomputing US tax until convergence (< $100 delta) or
     portfolio exhausted.
  4. Smart rebalance: sell overweight, buy underweight (target_vti /
     target_schd).
  5. Optional RSU `migrate_on_retirement` triggers the Taxable
     account’s per‑account rebalance strategy.
  6. Output `TransitionReport`.
- WHERE: `src/handlers/retirement_transition.rs:28-394`.
- OUTPUT: `SimResults.transition_report`.
- USER INPUT: `target_vti_pct`, `target_schd_pct`,
  `pre_funded_war_chest_jpy`, `pre_funded_bridge_jpy/usd`,
  `pre_funded_japan_tax_jpy`, `pre_funded_us_tax_usd`.

I.5  Roth IRA rebalance at age 59½
- WHAT: One‑shot. Liquidates non‑target assets; sells overweight VTI/SCHD;
  buys underweight proportionally. Tax‑free inside Roth.
- WHERE: `src/handlers/roth_rebalancer.rs`.
- RATIONALE / LAW: IRC §72(t) — qualified distributions allowed after
  age 59½; intra‑Roth trades are not taxable events.
- USER INPUT: `enable_roth_rebalance_at_59` toggle,
  `roth_rebalance_target_vti_pct`, `roth_rebalance_target_schd_pct`.

I.6  Tax‑loss harvesting (TLH)
- WHAT: In months listed in `tlh_active_months` (default [11, 12]) and
  only post‑retirement, scan each Taxable lot for a USD loss ≥
  `tlh_min_loss_usd`. Wash‑sale check: skip if a replacement lot was
  acquired within 30 calendar days before/after sale (§1091). Record
  recognised loss in both USD (offsets US cap gains) and JPY (rolls
  into `japan_loss_carryforward_jpy`).
- WHERE: `src/handlers/tax_loss_harvesting.rs:29-110`.
- RATIONALE / LAW: IRC §1091 (Wash‑sale disallowance); Japan IT Act Art.
  37‑12‑2 (3‑year listed‑security loss carry‑forward).
- ⚠ AUDIT NOTE — Per code comment, the §1091 wash‑sale rule does NOT
  apply to cryptocurrency (IRS Notice 2014‑21). Code still applies the
  guard conservatively for all assets; a crypto‑specific bypass could
  add a small accuracy gain.
- USER INPUT: `tlh_enabled`, `tlh_active_months`, `tlh_min_loss_usd`.

================================================================================
SECTION J — RSU 6013(g) NRA SPOUSE INTEGRATION (STAGE 02)
================================================================================

J.1  Spouse profile selection
- WHAT: One of `UsPerson` / `NraElectedToBeTreatedAsResident` /
  `NraMfs` / `NraHeadOfHouseholdEligible`. Drives effective filing
  status and which spouse income enters the US base.
- WHERE: `models/config.rs:407-425`; effective status derived in loader
  and at `controller.rs:842-858`.
- RATIONALE / LAW: IRC §6013(g) — election to treat NRA spouse as a
  resident. IRC §6013(h) — Year‑of‑Marriage election variant. Roth
  contribution phase‑out for MFS: IRC §408A(c)(3)(B)(ii), $0 – $10K MAGI.

J.2  §6013(g) income pooling
- WHAT: When profile = `NraElectedToBeTreatedAsResident`, add
  `spouse_japan_salary_jpy + spouse_japan_misc_income_jpy` (converted
  USD) to general‑basket ordinary; the Japan resident tax on that
  income enters the general FTC basket.
- WHERE: `controller.rs:841-858, 921`.
- USER INPUT: `spouse_profile`, `spouse_japan_salary_jpy`,
  `spouse_japan_misc_income_jpy` (UI 3333‑3403).

J.3  NRA‑MFS Roth phase‑out warning
- WHAT: In January, if profile = `NraMfs` AND `total_annual_compensation_usd
  > 10,000`, emit `RothMfsPhaseOutExceeded` SolvencyWarning and skip the
  Roth contribution.
- WHERE: `src/handlers/contributions.rs:70-91`.
- RATIONALE / LAW: IRC §408A(c)(3)(B)(ii) — MFS Roth phase‑out window
  $0 to $10,000 MAGI.

================================================================================
SECTION K — STAGE 06 REAL ESTATE & MORTGAGE
================================================================================

K.1  Monthly P&I payment
- WHAT: Standard fixed‑rate amortisation M = P · r·(1+r)^n / ((1+r)^n − 1).
- WHERE: `real_estate_engine.rs:14-23`. Balance: 29-40.

K.2  Property tax & rental income
- WHAT: `monthly_property_tax_jpy = annual / 12` (and USD); rental net =
  gross × (1 − vacancy) − insurance/12 − repairs_pct_fmv × FMV / 12.
- WHERE: `real_estate_engine.rs:60-92`.
- RATIONALE / LAW: 固定資産税 (Japan property tax) — 1.4% standard;
  user supplies annual amount.

K.3  Annual depreciation (tax)
- WHAT: JPY (declining‑balance simplified to straight‑line): purchase ×
  90% / life. Lives: Wood 22, RC 47, Steel 34, Other 22 years (法定耐用
  年数). USD: MACRS straight‑line — Residential 27.5y; Non‑residential
  39y; land assumed 20% (US).
- WHERE: `real_estate_engine.rs:128-153`.
- RATIONALE / LAW: 減価償却資産の耐用年数等に関する省令 (Japan); IRS
  MACRS §168.

K.4  HELOC available
- WHAT: `min(credit_line − outstanding, ltv_cap × fmv − mortgage)`.
- WHERE: `real_estate_engine.rs:104-114`.

K.5  Total real‑estate equity (snapshot)
- WHAT: `Σ fmv_jpy − Σ jpy_mortgage_balance − outstanding_heloc_usd × fx`;
  parallel USD version.
- WHERE: `real_estate_engine.rs:196-222`.
- OUTPUT: `AnnualSnapshot.real_estate_equity_jpy/usd`.

================================================================================
SECTION L — STAGE 07 ESTATE PLANNING
================================================================================

L.1  Japan 相続税 (Sōzoku‑zei) bracket table
- WHAT: Per‑heir share marginal rates: 10/15/20/30/40/45/50/55 %
  at ¥10M / ¥30M / ¥50M / ¥100M / ¥200M / ¥300M / ¥600M / ∞, with
  bracket‑deduction constants 0 / ¥500K / ¥2M / ¥7M / ¥17M / ¥27M /
  ¥42M / ¥72M.
- WHERE: `src/engine/tax/estate_tax.rs:17-39`.
- OUTPUT: Total tentative tax across statutory heirs.
- RATIONALE / LAW: 相続税法 第16条 (bracket schedule); NTA Calculation
  Sheet. Basic exclusion ¥30M + ¥6M × heir_count (相続税法 第15条).

L.2  Statutory heir shares & spousal deduction
- WHAT: Spouse 1/2; remaining children share equally (per
  `compute_japan_sozoku_zei` and `project_at_death`). Spousal 1/2
  deduction applied as a final 0.5 multiplier when a spouse is
  present.
- WHERE: `estate_tax.rs:222-249`.
- RATIONALE / LAW: 民法 第900条 (statutory shares); 相続税法 第19条の2
  (配偶者の税額軽減 — spouse pays the LARGER of statutory share or
  ¥160M).

L.3  US federal estate tax
- WHAT: Flat 40% on (estate − exclusion). Exclusion: TCJA era
  $13.61M (2024) × 1.028^years; post‑2026 sunset $7M × 1.028^years.
- WHERE: `estate_tax.rs:82-101`.
- RATIONALE / LAW: IRC §2001(c) — 40% top rate; §2010(c)(3) — basic
  exclusion amount. TCJA sunset per Pub. L. 115‑97 §11061.

L.4  US‑Japan estate treaty credit
- WHAT: `min(japan_paid × japan_situs_fraction, us_paid)`. Default
  `japan_situs_fraction = 1.0` for long‑term Japan residents.
- WHERE: `estate_tax.rs:120-127`.
- RATIONALE / LAW: US‑Japan Estate Tax Convention 1954 (Article VI
  pro‑rata credit). Conservative ceiling: cannot exceed US estate tax
  paid.

L.5  Lifetime gifting optimiser
- WHAT: Suggest ¥1.1M/recipient (暦年贈与) + $19K/recipient (§2503(b))
  for `years_remaining` until `death_date`; estimate Japan tax
  reduction by subtracting the projected total from estate.
- WHERE: `estate_tax.rs:154-184`.
- RATIONALE / LAW: 暦年贈与: 相続税法 第21条の5; US: IRC §2503(b).
- USER INPUT: `enable_estate_planning`, `death_date`, `heirs[]`,
  `enable_gifting_optimiser`, `annual_gift_jpy_per_recipient`,
  `gift_recipient_count`.

================================================================================
SECTION M — BUFFER / WAR‑CHEST / BRIDGE‑FUND LOGIC
================================================================================

M.1  War chest target & funding
- WHAT: JPY‑denominated cash reserve at retirement. Target =
  `war_chest_target_jpy` (or USD × FX). Funding timing:
  `AtRetirement` (lump sale at transition) or
  `GraduallyBeforeRetirement` (monthly skim over
  `war_chest_ramp_months`).
- WHERE: `retirement_transition.rs:49-62`,
  `contributions.rs:163-183` (gradual accumulation).
- USER INPUT: `war_chest_enabled`, `war_chest_target_jpy`,
  `war_chest_target_usd`, `war_chest_funding_timing`,
  `war_chest_ramp_months`.

M.2  Bridge fund target & funding
- WHAT: Months of base spend × `bridge_months_target` (max with NHI
  spike buffer = `nhi_spike_monthly_jpy × MEDICAL_BUFFER_MONTHS (18)`).
  Funding: `AtRetirement` or `GraduallyBeforeRetirement` over
  `bridge_fund_ramp_months`.
- WHERE: `retirement_transition.rs:64-93`,
  `contributions.rs:185-215`.

M.3  Surplus deposit rules
- WHAT: After spending, JPY surplus refills War Chest up to gap; USD
  surplus refills Bridge Fund up to `bridge_months_target` USD; excess
  buys VTI (`target_vti_pct`) / SCHD (`target_schd_pct`) in Taxable.
- WHERE: `cashflow_manager.rs:637-674`.
- USER INPUT: `target_vti_pct`, `target_schd_pct`.

================================================================================
SECTION N — ACCOUNT‑LEVEL TAX FLAGS (§5.1 ROUTING)
================================================================================

N.1  AccountJurisdiction
- WHAT: `Us` / `Japan` / `Both` / `None`. Determines which tax engines
  consult that account’s dividends/gains. Roth typically = Both with
  `us_tax_advantaged = true`; iDeCo/NISA = Japan with
  `japan_tax_advantaged = true`.
- WHERE: `models/assets.rs:5-29`; gates at `dividends.rs:128-144`.
- USER INPUT: Per‑account combo box (UI 2608).

N.2  Per‑account rebalance strategy (V7.7)
- WHAT: Each Account can carry an optional `AccountRebalanceStrategy {
  targets: Vec<RebalanceTarget>, frequency_months,
  trigger_year_month, is_one_time, enabled }`. Strategy fires when
  current date crosses trigger. Tax estimated at LTCG 15% / STCG 22%
  (US) and 20.315% (JP) based on `AccountJurisdiction`.
- WHERE: `handlers/rebalancing.rs:114-214`.

================================================================================
SECTION O — MONTE CARLO (STAGE 08)
================================================================================

O.1  Correlated paths
- WHAT: When `mc_use_correlated_paths = true`, use multivariate normal
  draws based on `mc_correlation_matrix` to model the “safe‑haven yen”
  effect (negative correlation USD/JPY ↔ US Equity).
- WHERE: `src/simulation/monte_carlo.rs` (full module).
- USER INPUT: `mc_use_correlated_paths` + correlation pairs in JSON.

================================================================================
SECTION P — REPORTING & SNAPSHOTS
================================================================================

P.1  AnnualSnapshot
- WHAT: 60+ fields captured every Dec 31. Includes US/JP tax bills,
  cap gains, FTC carryover, real‑estate equity, PFIC MTM phantom
  income, Kaigo premium, gift sink, exit‑tax trigger, estate summary
  (terminal year only).
- WHERE: `src/models/snapshot.rs`; populated at
  `controller.rs:1007-1133`.

P.2  CSV / UI output
- WHERE: `src/reporter.rs`; UI panels in `src/ui/panels/`.

================================================================================
SECTION Q — USER INPUTS MASTER REFERENCE
================================================================================

The Input Panel (`src/ui/panels/input_panel.rs`) organises inputs into
the following sections. Each entry: control → config field → engine
impact.

Q.1  Scenario File
- Load / Save / New buttons → loads `Config` + accounts.

Q.2  Rebalance Schedule (view-only)
- Surfaces upcoming RSU vest dates, retirement, rebalance, FX shocks.

Q.3  Economics
- `usd_jpy`, `inflation_cola`, `inflation_japan`, `ira_limit_growth`,
  `fx_drift_*` (cadence vs continuous).

Q.4  Japan Resident Region
- Prefecture dropdown → `cfg.prefecture` → lookup table for
  `(income_rate, per_capita_jpy)`. City field — Nagoya gets 9.7%.

Q.5  Tax Jurisdiction (global)
- `tax_jurisdiction = Both / UsOnly / JapanOnly / TaxFree`. Gates entire
  tax engines (Japan engine bypassed in UsOnly; US engine bypassed in
  JapanOnly).

Q.6  US Tax Mitigation
- `us_tax_strategy = FtcOnly | FeieAndFtc`. Drives the
  `calculate_liability_with_basket_ftc` vs
  `calculate_liability_with_feie_ftc` branch.

Q.7  US Filing Status
- Dropdown → `tax_rules = TaxRules::for_filing_status(...)`.
- Affects std_deduction, brackets, LTCG limits, NIIT threshold,
  senior add‑on.

Q.8  US State
- Dropdown → `us_state_code`, `us_state_rate` (auto from table).

Q.9  NHI Premium Model
- Radio: Calculated vs ManualOverride vs NinkiKeizoku. Calculated:
  9 numeric overrides + `include_us_investment_income` toggle.
  ManualOverride: spike + ongoing totals. NinkiKeizoku: monthly + duration
  + fallback.

Q.10  Long‑Term Care Insurance
- `kaigo_hoken_enabled` checkbox + `kaigo_care_scenario` dropdown
  (None / Low / Medium / High).

Q.11  Investment Accounts
- Per‑account: name, jurisdiction, `us_tax_advantaged`,
  `japan_tax_advantaged`, currency, location. For DC: monthly contribution,
  fund allocation, total‑return %. For Taxable/Roth: positions
  (ticker, units, price, cost basis, JPY basis, growth%, DRIP, asset
  class, return profile, PFIC regime).

Q.12  Family Demographics
- `is_married`, spouse birth date, `spouse_profile`, dependent
  birthdays, `is_college_student`, `monthly_dependent_precision`,
  `jido_teate_enabled`.

Q.13  VA Disability
- Rating dropdown, dependent status, VA monthly override,
  SMC variant, SMC override.

Q.14  FERS
- `fers_monthly_start`, `fers_start_date`, `fers_jurisdiction`,
  `fers_japan_local_tax_exempt`.

Q.15  Social Security
- `ss_monthly_usd`, `ss_start_age`, `ss_jurisdiction`. Spouse:
  `spouse_ss_monthly_usd`, `_start_age`, `_jurisdiction`.

Q.16  Nenkin
- `nenkin_income_monthly_jpy`, `_start_age`. Spouse counterpart.

Q.17  War Chest & Bridge Fund
- Enabled checkboxes, currency, target amount, funding timing
  (At Retirement vs Gradually), ramp months.

Q.18  Education Fund + Gift Sink
- `enable_education_savings`, `edu_savings_jpy_monthly`,
  `enable_gift_sink`, `annual_gift_jpy_per_recipient`,
  `gift_recipient_count`, `us_gift_exclusion_usd`.

Q.19  Estate Planning
- `enable_estate_planning`, `death_date`, `heirs[]`,
  `enable_gifting_optimiser`.

Q.20  Recession & FX Shock
- `recession_enabled`, `recession_events`, `fx_shock_events`,
  `shock_ordering` (DepreciateThenReprice / RepriceThenDepreciate /
  Simultaneous).

Q.21  RSU
- `rsu_tax_handling` (SALARY / SELL_TO_COVER),
  `rsu_sell_to_cover_realism`, `rsu_sell_to_cover_policy`. Awards
  table.

Q.22  Withdrawal Strategy
- `withdrawal_strategy` (DividendOnly / TotalReturn / Hybrid),
  `withdrawal_waterfall` (Defensive / Cautious),
  `withdrawal_regime` (Shielded / Dynamic), `fx_spread_penalty`.

Q.23  Stress / Audit toggles
- `track_pfic_basis_drift`, `crypto_tax_enabled`,
  `monthly_dependent_precision`, `exit_tax_include_tax_advantaged`,
  `japan_residency_start_date`.

================================================================================
SECTION R — KNOWN MODELING GAPS & POSSIBLE DIVERGENCES FROM CURRENT LAW
================================================================================

For audit prioritisation. Each item marked ⚠ above is restated here.

R.1  NIIT threshold (B.4) — Code inflates the $250K MFJ threshold; statute
   (IRC §1411) does NOT index it. Probable over‑inflation drift.

R.2  §904 FTC carryover — modeled as 1 year, statute is 10 years forward
   (and 1 year backward).

R.3  Japan capital‑loss carry‑forward (C.6) — modeled as indefinite
   rolling; statute is 3 years (租特法 第37条の12の2).

R.4  Reconstruction surcharge (C.8) — modeled forever; sunsets FY2037
   per 復興特別所得税法.

R.5  SSDI Combined Income thresholds (B.8) — hard‑coded MFJ $32K/$44K;
   may misstate for Single/MFS/HoH ($25K/$34K).

R.6  Tax brackets — `TaxRules::default()` ships 2026‑ish MFJ values;
   verify against 2026 IRS Rev. Proc. and confirm Single/MFS/HoH
   tables in `for_filing_status` match the same year.

R.7  Kaigo Hoken brackets — Sagamihara defaults marked “illustrative”;
   verify with actual 令和8年度 municipal notice.

R.8  Jido Teate (F.2) — no income cap modeled; the cap was abolished
   effective 2024‑10‑01 so the model is correct for 2024+ but should
   be revisited if any prior‑year backtest is added.

R.9  Spousal 1/2 deduction (L.2) — Engine multiplies the WHOLE estate
   tax by 0.5 if a spouse exists. Statutory rule is the larger of
   spouse statutory share OR ¥160M (相続税法 第19条の2). For mid‑sized
   estates the engine may under‑deduct.

R.10  TCJA sunset (L.3) — Hard‑coded $7M post‑2026 figure; if Congress
   extends or modifies TCJA, this needs an update.

R.11  US estate tax: state estate taxes NOT modeled.

R.12  Wash‑sale on crypto (I.6) — guard applied conservatively; IRS
   Notice 2014‑21 says §1091 does not apply to crypto.

R.13  Treaty Article 18 toggle (F.7) — modeled as an on/off switch; the
   actual treaty has nuance around government vs private‑sector
   employees and Saving Clause carve‑outs.

R.14  Resident tax dependent thresholds (C.3) — code uses ¥9M / ¥9.5M /
   ¥10M phase‑outs of ¥330K → ¥220K → ¥110K → 0. These are the 2020+
   amounts; older code paths may have used the pre‑2018 ¥380K figure.

R.15  FEIE limit (B.7) — hard‑coded 2026 $126,500; needs annual update.

R.16  VA rates (F.3) — 2026 official rates ship; COLA forward from
   2026 baseline. The dependent child cutoff is precise to day, but
   the schoolchild add‑on (over‑18 enrolled) is NOT modeled per code
   comment.

================================================================================
END OF REPORT
================================================================================
