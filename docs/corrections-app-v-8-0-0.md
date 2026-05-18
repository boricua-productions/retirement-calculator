# **Engineering Specification Document**

**Target Version:** Stage 13 Build (v8.0.0-alpha)

**Subject:** Remediation of Regulatory Gaps, Architectural Tax-Routing Bugs, and 2026 Legal Compliance Framework (US/Japan)

**Status:** Approved for Implementation

## ---

**Section 1: Global Simulation Axis & Timing Controls**

### **1.1 Resident Tax Collection Cadence Logic**

* **Location:** src/simulation/controller.rs (handle\_new\_year, process\_month)  
* **Context:** Currently, the engine schedules Japanese Resident Tax liabilities exclusively via *Futsu Choushuu* (Ordinary Collection) in 4 quarterly chunks (June, August, October, January).  
* **Specification Change:** Implement a state gate based on the user's employment status flag (state.is\_retired).  
  * **Active Phase (state.is\_retired \== false):** Resident tax must follow *Tokubetsu Choushuu* (Special Collection). Amortize the calculated annual resident tax liability evenly across **12 monthly installments** running from June of the assessment year through May of the following calendar year. Pushed to the cashflow waterfall as a fixed monthly ExpenseRule.  
  * **Retired Phase (state.is\_retired \== true):** Revert to the legacy 4-quarter installment cycle (*Futsu Choushuu*) due in June, August, October, and January.

### **1.2 Global Parameter Baseline Synchronisation (2026)**

* **Location:** models/config.rs (TaxRules::default())  
* **Specification Change:** Hardcode the base calibration figures for the 2026 execution environment. Overwrite all legacy values with the following non-indexed absolute parameters:

| Parameter Key | Reference Law / Section | 2026 Baseline Value |
| :---- | :---- | :---- |
| feie\_annual\_limit\_usd | IRC §911 | **$132,900** |
| niit\_joint\_threshold\_usd | IRC §1411 | **$250,000** (Frozen, non-indexed) |
| niit\_single\_threshold\_usd | IRC §1411 | **$200,000** (Frozen, non-indexed) |
| us\_estate\_exclusion\_base | 2026 Unified Credit Guidelines | **$15,000,000** |

## ---

**Section 2: US Federal & State Tax Engine Updates**

### **2.1 Standard Deduction & Filing Status Correction**

* **Module:** src/engine/tax/us\_tax.rs (TaxRules::for\_filing\_status)  
* **Specification Change:** Replace the synthetic inflated legacy values with exact statutory numbers for the 2026 tax year.

Rust

pub fn for\_filing\_status(status: FilingStatus) \-\> Self {  
    match status {  
        FilingStatus::MarriedFilingJointly \=\> TaxRules {  
            std\_deduction: 32200.0,  
            ltcg\_0\_limit: 115000.0,  
            // progressive ordinary income brackets...  
        },  
        FilingStatus::Single | FilingStatus::MarriedFilingSeparately \=\> TaxRules {  
            std\_deduction: 16100.0,  
            ltcg\_0\_limit: 47025.0,  
            // progressive ordinary income brackets...  
        },  
        FilingStatus::HeadOfHousehold \=\> TaxRules {  
            std\_deduction: 24150.0,  
            ltcg\_0\_limit: 63000.0,  
            // progressive ordinary income brackets...  
        },  
    }  
}

### **2.2 Section 70103 Enhanced Senior Deduction Engine**

* **Module:** src/engine/tax/us\_tax.rs  
* **Specification Change:** Introduce a new compliance calculation layer to process the temporary Enhanced Senior Deduction. This layer overrides the legacy baseline age additions.  
* **Mathematical Logic:** If a taxpayer or covered spouse reaches age 65 on or before December 31 of the simulation year, calculate an additive deduction pool of **$6,000 per eligible individual** (maximum $12,000 for MFJ). Implement a phase-out sequence with a 6% clawback rate for Modified Adjusted Gross Income (MAGI) exceeding baseline thresholds.

$$\\text{Deduction}\_{\\text{reduced}} \= \\text{Deduction}\_{\\text{max}} \- \\left( (\\text{MAGI} \- \\text{Threshold}) \\times 0.06 \\right)$$

* **Threshold Config:**  
  * MFJ: Phase-out begins at **$150,000 MAGI**; drops to a floor of $0 at $350,000 MAGI.  
  * Single / MFS / HoH: Phase-out begins at **$75,000 MAGI**; drops to a floor of $0 at $175,000 MAGI.

### **2.3 SSDI & SS Joint Combined Income Multi-Bracket Selector**

* **Module:** src/engine/tax/us\_tax.rs (ssdi\_combined\_income\_taxable\_portion)  
* **Specification Change:** Eliminate the structural defect that forces Married Filing Jointly (MFJ) threshold ranges onto alternative filing profiles. Incorporate an exhaustive matching block to change threshold boundaries dynamically based on user config profiles:

Rust

let (low\_threshold, high\_threshold) \= match cfg.tax\_rules.filing\_status {  
    FilingStatus::MarriedFilingJointly \=\> (32000.0, 44000.0),  
    FilingStatus::Single | FilingStatus::HeadOfHousehold \=\> (25000.0, 34000.0),  
    FilingStatus::MarriedFilingSeparately \=\> {  
        if state.lived\_with\_spouse\_during\_year { (0.0, 0.0) } else { (25000.0, 34000.0) }  
    }  
};

### **2.4 Bounded IRC §904(c) Baskets & Foreign Tax Credit Carryover Queue**

* **Module:** src/engine/tax/us\_tax.rs / src/models/snapshot.rs  
* **Specification Change:** Deprecate the unbound, single-year rolling field variables state.ftc\_carryover\_passive\_usd and state.ftc\_carryover\_general\_usd. Replace them with FIFO tracking buffers to accurately capture the statutory 10-year limit.

Rust

\#\[derive(Serialize, Deserialize, Clone, Debug)\]  
pub struct FtcCarryoverQueue {  
    pub passive\_basket: Vec\<FtcLot\>,  
    pub general\_basket: Vec\<FtcLot\>,  
}

\#\[derive(Serialize, Deserialize, Clone, Debug)\]  
pub struct FtcLot {  
    pub origin\_year: u16,  
    pub remaining\_credit\_usd: f64,  
}

* **Execution Rule:** At each year-end true-up step, pass unused credits into their respective basket vectors. Evict any FtcLot where current\_year \- origin\_year \> 10\.

## ---

**Section 3: Japan Tax Engine & Portfolio Liquidation Updates**

### **3.1 Total Moving Average Portfolio Accounting (The Japan Mandate)**

* **Module:** src/handlers/cashflow\_manager.rs (v7\_liquidate\_for\_deficit), src/handlers/dividends.rs  
* **Specification Change:** Stop using the US specific lot identification matching policy (Highest-Basis-First / HIFO) for Japanese realized gain assessments. Japanese law demands a pooled cost assignment tracking logic across identical holdings.  
* **Calculation Engine Implementation:** Maintain a secondary, background JPY ledger for every asset class position. When additional tranches are acquired, recalculate the aggregate pool valuation using the formal moving average method (*総平均法に準ずる方法*):

$$\\text{Weighted Average Cost Basis (JPY)} \= \\frac{\\sum (\\text{Purchase Price in USD} \\times \\text{FX at Purchase})}{\\text{Total Shares Owned}}$$

* **Liquidation Step:** When Tier 8 triggers asset liquidations to patch currency shortfalls, the reportable taxable gain inside Japan must strictly scale off this uniform average JPY basis calculation, irrespective of which distinct historical asset slots are unmapped for US tax optimization.

### **3.2 US State Tax Entry into Foreign Tax Credit Pool**

* **Module:** src/engine/tax/japan\_tax.rs  
* **Specification Change:** Modify the calculation routines for the Japanese Foreign Tax Credit (*外国税額控除*). Read the output calculated from state-level income processing engines (state\_tax). This value must be systematically aggregated into the total deductible foreign tax ceiling array along with US federal metrics to prevent artificial liquidity drag.

### **3.3 Statutory Capital Loss Rolling Constraint (3-Year Cap)**

* **Module:** src/simulation/controller.rs (handle\_new\_year)  
* **Specification Change:** Implement a rigorous array tracking structure for state.japan\_loss\_carryforward\_jpy to mirror *租税特別措置法 第37条の12の2*. Replace the legacy boundless scalar balance.

Rust

// Track precisely the last 3 assessment periods  
pub struct JapanLossLedger {  
    pub year\_minus\_1: f64,  
    pub year\_minus\_2: f64,  
    pub year\_minus\_3: f64,  
}

* **Yearly Rolling Loop Execution:** At the month=1 rollover checkpoint, discard year\_minus\_3. Shift current balances down (year\_minus\_2 replaces year\_minus\_3, etc.), and initialize the incoming slot with the net capital losses realized from the prior December timeline.

### **3.4 Reconstruction Surcharge Sunset Boundary**

* **Module:** src/engine/tax/japan\_tax.rs (calculate\_income\_tax)  
* **Specification Change:** Incorporate a dynamic assessment date filter to terminate the calculation of the special reconstruction income tax (*復興特別所得税*).  
* **Logic Rule:** If current\_simulation\_year \<= 2037, apply the standard regulatory surcharge scalar multiplier of 1.021. If current\_simulation\_year \>= 2038, bypass the modifier block completely, locking the calculation multiplier baseline at exactly 1.000.

### **3.5 Visa Classification Filter for Exit Tax Triggers**

* **Module:** src/simulation/controller.rs (evaluate\_exit\_tax\_trigger)  
* **Specification Change:** Add a critical residency exemption layer to prevent false-positive trigger events on high-net-worth foreign professionals.  
* **Implementation Steps:**  
  1. Add an active categorical profile parameter to global asset inputs:  
     cfg.family\_unit.primary\_taxpayer\_visa: VisaType  
  2. Add the corresponding evaluation matching variants:  
     enum VisaType { Table1(WorkingDirector), Table2(PermanentOrSpouse) }  
  3. Modify the processing execution code: If the config field evaluates to VisaType::Table1, short-circuit the execution block and force-return exit\_tax\_triggered \= false. Time accrued under a Table 1 structural status must be completely excluded from the 5-out-of-10-year exit tax threshold tracking calculation.

## ---

**Section 4: Estate Planning & Inheritance Tax Reforms**

### **4.1 Japan Spousal Tax Mitigation Ledger (Art. 19-2 Compliance)**

* **Module:** src/engine/tax/estate\_tax.rs (compute\_japan\_sozoku\_zei)  
* **Specification Change:** Completely deprecate the basic 0.5 global scaling shortcut used to simulate spousal relief. Implement a full statutory allocation block that reflects Japanese Inheritance Tax Law Article 19-2 (*配偶者の税額軽減*).  
* **Algorithmic Resolution Path:**  
  1. Calculate total combined tentative inheritance liability across all statutorily defined family heirs.  
  2. Isolate the explicit asset fraction assigned to the surviving spouse.  
  3. Calculate the spousal tax exemption credit equivalent to the exact tax due on the **larger** of the following two boundaries:  
     * The spouse's statutory distribution share (typically 50% of total estate value).  
     * An absolute asset ceiling value of **¥160,000,000**.  
  4. Subtract this computed credit directly from the spouse’s personal tax liability line item, ensuring alternate remaining heir liability weights remain unaffected.

## ---

**Section 5: Order of Execution & Invariants**

To avoid processing discrepancies, multi-currency rounding drops, or cash shortfall bugs, code loop handlers inside src/simulation/controller.rs must rigidly follow this year-end schedule during the December processing phase:

┌─────────────────────────────────────────────────────────────┐  
│ 1\. Process Monthly Distributions & Realized Capital Events  │  
└──────────────────────────────┬──────────────────────────────┘  
                               │  
                               ▼  
┌─────────────────────────────────────────────────────────────┐  
│ 2\. Execute Tax-Loss Harvesting Engine                       │  
│    (Wash-sale check via §1091, skip crypto per Notice 2014\) │  
└──────────────────────────────┬──────────────────────────────┘  
                               │  
                               ▼  
┌─────────────────────────────────────────────────────────────┐  
│ 3\. Compute Year-End Tax True-Up Liabilities                │  
│    (Process stacked brackets \+ Section 70103 updates)       │  
└──────────────────────────────┬──────────────────────────────┘  
                               │  
                               ▼  
┌─────────────────────────────────────────────────────────────┐  
│ 4\. Route Liabilities into Cashflow Waterfall Tiers          │  
│    (Drain remaining cash balances before system snapshots)  │  
└─────────────────────────────────────────────────────────────┘

* **System Invariant 1:** The engine must never process portfolio rebalancing distributions or liquidation events for a year until all incoming dividend components (ROC basis tracking, special allocations) have completed their execution passes for that period.  
* **System Invariant 2:** All calculated federal, state, and local true-up tax liabilities must feed back into the cashflow waterfall as prioritizing expenses *before* the month closes. This guarantees that buffer balances accurately maintain the cash liquidity needed for upcoming tax obligations.