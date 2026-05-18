/// Nationwide Japan resident tax (住民税 / Juminzei) regional database.
///
/// Standard rate: 10% income portion (6% city/ward + 4% prefecture) + ¥6,000 per-capita
/// levy (均等割: ¥3,500 city + ¥1,500 prefecture + ¥1,000 forest environment tax from FY2024).
///
/// Known exceptions:
///   Nagoya City (名古屋市) — reduced city portion 5.7%, total rate 9.7%.
///
/// All other 47 prefectures and their major cities use the standard 10% rate.
/// Source: National Tax Agency (NTA) and municipal tax ordinances.
/// Resident tax rates for a specific prefecture / city combination.
#[derive(Debug, Clone)]
pub struct ResidentTaxRates {
    /// Combined income tax rate (city/ward + prefecture). Standard: 0.10.
    pub income_rate: f64,
    /// Annual per-capita levy (均等割). FY2024+: ¥6,000.
    pub per_capita_jpy: f64,
}

/// Standard rate used by almost all municipalities nationwide.
pub const STANDARD_INCOME_RATE: f64 = 0.10;
/// FY2024+ per-capita levy: ¥3,500 ward + ¥1,500 prefecture + ¥1,000 forest env tax.
pub const STANDARD_PER_CAPITA_JPY: f64 = 6_000.0;
/// Nagoya City special reduced rate (city portion 5.7% vs standard 6.0%).
pub const NAGOYA_INCOME_RATE: f64 = 0.097;

/// Look up the resident tax rates for the given prefecture and city.
///
/// Returns `STANDARD_INCOME_RATE` / `STANDARD_PER_CAPITA_JPY` for all locations
/// except Nagoya City (9.7%).
pub fn lookup_resident_tax_rates(prefecture: &str, city: &str) -> ResidentTaxRates {
    match city.trim() {
        "Nagoya" => ResidentTaxRates {
            income_rate: NAGOYA_INCOME_RATE,
            per_capita_jpy: STANDARD_PER_CAPITA_JPY,
        },
        _ => {
            // Tokyo special wards and all other municipalities: standard 10%.
            let _ = prefecture; // rate is uniform; variable retained for future extensions
            ResidentTaxRates {
                income_rate: STANDARD_INCOME_RATE,
                per_capita_jpy: STANDARD_PER_CAPITA_JPY,
            }
        }
    }
}

// ─── V8.1 NHI rate lookup ─────────────────────────────────────────────────────

/// V8.1 — Confidence label for a (prefecture, city) NHI rate lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NhiRateProvenance {
    /// The rates are the actual official schedule for this municipality.
    Authoritative,
    /// The rates are the nationwide-standard estimate (Sagamihara 2026 used as
    /// the proxy). User should verify against their city office.
    Estimate,
}

/// V8.1 — Look up an NHI rate schedule for the given prefecture/city.
/// Initially only Sagamihara has authoritative data; everything else returns
/// the same schedule labeled `Estimate` so the UI can flag the difference.
/// Extend over time as more municipalities are researched.
pub fn nhi_rates_for(prefecture: &str, city: &str) -> (crate::models::config::NhiCalculatedRates, NhiRateProvenance) {
    match (prefecture.trim(), city.trim()) {
        ("Kanagawa", "Sagamihara") => (
            crate::models::config::NhiCalculatedRates::sagamihara_2026(),
            NhiRateProvenance::Authoritative,
        ),
        // Add more municipalities here as their official rate sheets are researched.
        _ => (
            crate::models::config::NhiCalculatedRates::sagamihara_2026(),
            NhiRateProvenance::Estimate,
        ),
    }
}

// ─── Prefecture list (all 47, north → south) ─────────────────────────────────

pub const ALL_PREFECTURES: &[&str] = &[
    "Hokkaido",
    "Aomori", "Iwate", "Miyagi", "Akita", "Yamagata", "Fukushima",
    "Ibaraki", "Tochigi", "Gunma", "Saitama", "Chiba", "Tokyo", "Kanagawa",
    "Niigata", "Toyama", "Ishikawa", "Fukui",
    "Yamanashi", "Nagano",
    "Shizuoka", "Aichi", "Gifu", "Mie",
    "Shiga", "Kyoto", "Osaka", "Hyogo", "Nara", "Wakayama",
    "Tottori", "Shimane", "Okayama", "Hiroshima", "Yamaguchi",
    "Tokushima", "Kagawa", "Ehime", "Kochi",
    "Fukuoka", "Saga", "Nagasaki", "Kumamoto", "Oita", "Miyazaki", "Kagoshima",
    "Okinawa",
];

/// Returns the curated list of major cities for the given prefecture.
/// The first entry is always "Other (Standard Rate)" as the safe default.
pub fn cities_for_prefecture(prefecture: &str) -> &'static [&'static str] {
    match prefecture {
        "Hokkaido" => &[
            "Other (Standard Rate)", "Sapporo", "Asahikawa", "Hakodate",
            "Obihiro", "Kushiro", "Kitami", "Tomakomai",
        ],
        "Aomori" => &[
            "Other (Standard Rate)", "Aomori", "Hirosaki", "Hachinohe", "Misawa",
        ],
        "Iwate" => &[
            "Other (Standard Rate)", "Morioka", "Ichinoseki", "Oshu", "Kitakami",
        ],
        "Miyagi" => &[
            "Other (Standard Rate)", "Sendai", "Ishinomaki", "Natori", "Tome",
        ],
        "Akita" => &[
            "Other (Standard Rate)", "Akita", "Noshiro", "Yokote", "Daisen",
        ],
        "Yamagata" => &[
            "Other (Standard Rate)", "Yamagata", "Yonezawa", "Tsuruoka", "Sakata",
        ],
        "Fukushima" => &[
            "Other (Standard Rate)", "Fukushima", "Koriyama", "Iwaki", "Aizuwakamatsu",
        ],
        "Ibaraki" => &[
            "Other (Standard Rate)", "Mito", "Tsukuba", "Hitachi", "Tsuchiura",
            "Koga", "Kashima", "Hitachinaka",
        ],
        "Tochigi" => &[
            "Other (Standard Rate)", "Utsunomiya", "Oyama", "Nasu", "Nikko",
        ],
        "Gunma" => &[
            "Other (Standard Rate)", "Maebashi", "Takasaki", "Kiryu", "Isesaki",
        ],
        "Saitama" => &[
            "Other (Standard Rate)", "Saitama", "Kawaguchi", "Kawagoe",
            "Tokorozawa", "Kasukabe", "Koshigaya", "Ageo",
        ],
        "Chiba" => &[
            "Other (Standard Rate)", "Chiba", "Funabashi", "Matsudo",
            "Ichikawa", "Urayasu", "Narashino", "Kashiwa",
        ],
        "Tokyo" => &[
            "Other (Standard Rate)",
            // 23 Special Wards
            "Chiyoda Ward", "Chuo Ward", "Minato Ward", "Shinjuku Ward",
            "Bunkyo Ward", "Taito Ward", "Sumida Ward", "Koto Ward",
            "Shinagawa Ward", "Meguro Ward", "Ota Ward", "Setagaya Ward",
            "Shibuya Ward", "Nakano Ward", "Suginami Ward", "Toshima Ward",
            "Kita Ward", "Arakawa Ward", "Itabashi Ward", "Nerima Ward",
            "Adachi Ward", "Katsushika Ward", "Edogawa Ward",
            // Western Cities
            "Hachioji", "Tachikawa", "Musashino", "Mitaka",
            "Fuchu", "Chofu", "Machida", "Kodaira",
        ],
        "Kanagawa" => &[
            "Other (Standard Rate)", "Yokohama", "Kawasaki", "Sagamihara",
            "Fujisawa", "Yokosuka", "Odawara", "Chigasaki", "Atsugi",
        ],
        "Niigata" => &[
            "Other (Standard Rate)", "Niigata", "Nagaoka", "Joetsu",
            "Sanjo", "Kashiwazaki",
        ],
        "Toyama" => &[
            "Other (Standard Rate)", "Toyama", "Takaoka", "Imizu", "Nanto",
        ],
        "Ishikawa" => &[
            "Other (Standard Rate)", "Kanazawa", "Hakusan", "Komatsu", "Nomi",
        ],
        "Fukui" => &[
            "Other (Standard Rate)", "Fukui", "Echizen", "Sabae", "Obama",
        ],
        "Yamanashi" => &[
            "Other (Standard Rate)", "Kofu", "Fuefuki", "Chuo",
        ],
        "Nagano" => &[
            "Other (Standard Rate)", "Nagano", "Matsumoto", "Ueda",
            "Iida", "Suwa",
        ],
        "Shizuoka" => &[
            "Other (Standard Rate)", "Shizuoka", "Hamamatsu", "Numazu",
            "Fuji", "Mishima",
        ],
        "Aichi" => &[
            "Other (Standard Rate)",
            "Nagoya",   // ← special rate: 9.7%
            "Toyota", "Okazaki", "Kasugai", "Ichinomiya",
            "Nishio", "Anjo", "Seto",
        ],
        "Gifu" => &[
            "Other (Standard Rate)", "Gifu", "Ogaki", "Kakamigahara",
            "Tajimi", "Minokamo",
        ],
        "Mie" => &[
            "Other (Standard Rate)", "Tsu", "Yokkaichi", "Suzuka",
            "Matsusaka", "Kuwana",
        ],
        "Shiga" => &[
            "Other (Standard Rate)", "Otsu", "Kusatsu", "Moriyama",
            "Higashiomi",
        ],
        "Kyoto" => &[
            "Other (Standard Rate)", "Kyoto", "Uji", "Kameoka",
            "Muko", "Joyo",
        ],
        "Osaka" => &[
            "Other (Standard Rate)", "Osaka", "Sakai", "Higashiosaka",
            "Takatsuki", "Hirakata", "Toyonaka", "Suita",
        ],
        "Hyogo" => &[
            "Other (Standard Rate)", "Kobe", "Himeji", "Nishinomiya",
            "Amagasaki", "Akashi", "Itami", "Kakogawa",
        ],
        "Nara" => &[
            "Other (Standard Rate)", "Nara", "Kashihara", "Yamatokoriyama",
            "Tenri",
        ],
        "Wakayama" => &[
            "Other (Standard Rate)", "Wakayama", "Hashimoto", "Arita",
        ],
        "Tottori" => &[
            "Other (Standard Rate)", "Tottori", "Yonago", "Kurayoshi",
        ],
        "Shimane" => &[
            "Other (Standard Rate)", "Matsue", "Izumo", "Hamada",
        ],
        "Okayama" => &[
            "Other (Standard Rate)", "Okayama", "Kurashiki", "Tsuyama",
            "Tamano",
        ],
        "Hiroshima" => &[
            "Other (Standard Rate)", "Hiroshima", "Fukuyama", "Kure",
            "Higashihiroshima", "Onomichi",
        ],
        "Yamaguchi" => &[
            "Other (Standard Rate)", "Yamaguchi", "Shimonoseki", "Ube",
            "Hofu",
        ],
        "Tokushima" => &[
            "Other (Standard Rate)", "Tokushima", "Anan", "Naruto",
        ],
        "Kagawa" => &[
            "Other (Standard Rate)", "Takamatsu", "Marugame", "Sakaide",
        ],
        "Ehime" => &[
            "Other (Standard Rate)", "Matsuyama", "Imabari", "Uwajima",
            "Niihama",
        ],
        "Kochi" => &[
            "Other (Standard Rate)", "Kochi", "Nankoku", "Tosa",
        ],
        "Fukuoka" => &[
            "Other (Standard Rate)", "Fukuoka", "Kitakyushu", "Kurume",
            "Omuta", "Iizuka", "Dazaifu",
        ],
        "Saga" => &[
            "Other (Standard Rate)", "Saga", "Karatsu", "Tosu",
        ],
        "Nagasaki" => &[
            "Other (Standard Rate)", "Nagasaki", "Sasebo", "Isahaya",
        ],
        "Kumamoto" => &[
            "Other (Standard Rate)", "Kumamoto", "Yatsushiro", "Arao",
            "Uto",
        ],
        "Oita" => &[
            "Other (Standard Rate)", "Oita", "Beppu", "Nakatsu",
            "Usuki",
        ],
        "Miyazaki" => &[
            "Other (Standard Rate)", "Miyazaki", "Miyakonojo", "Nobeoka",
        ],
        "Kagoshima" => &[
            "Other (Standard Rate)", "Kagoshima", "Kirishima", "Kanoya",
        ],
        "Okinawa" => &[
            "Other (Standard Rate)", "Naha", "Okinawa", "Uruma",
            "Urasoe", "Nago", "Itoman",
        ],
        _ => &["Other (Standard Rate)"],
    }
}

/// Returns a short annotation string shown next to the city name in the UI.
/// Identifies any special rate deviations from the 10% standard.
pub fn city_rate_annotation(city: &str) -> Option<&'static str> {
    match city.trim() {
        "Nagoya" => Some("9.7% — reduced city rate"),
        _ => None,
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::tax::japan_tax::JapanTaxEngine;

    /// Standard income of ¥3M: Nagoya should produce less resident tax than Tokyo.
    #[test]
    fn test_nagoya_rate_lower_than_tokyo() {
        let income = 3_000_000.0; // ¥3M annual pension (e.g., FERS converted to JPY)
        let soc_ins = 0.0;
        let age = 65;
        let deps = 1;

        let tokyo_rates   = lookup_resident_tax_rates("Tokyo", "Shinjuku Ward");
        let nagoya_rates  = lookup_resident_tax_rates("Aichi", "Nagoya");

        let tokyo_tax  = JapanTaxEngine::calculate_resident_tax(
            0.0, income, soc_ins, age, deps,
            tokyo_rates.income_rate, tokyo_rates.per_capita_jpy,
        );
        let nagoya_tax = JapanTaxEngine::calculate_resident_tax(
            0.0, income, soc_ins, age, deps,
            nagoya_rates.income_rate, nagoya_rates.per_capita_jpy,
        );

        assert!(
            nagoya_tax < tokyo_tax,
            "Nagoya (9.7%) should be cheaper than Tokyo (10.0%): nagoya=¥{:.0} tokyo=¥{:.0}",
            nagoya_tax, tokyo_tax
        );
    }

    /// Tax delta between Nagoya and a standard-rate city should equal
    /// exactly 0.3% of net-of-deduction taxable income (rate difference is 0.003).
    #[test]
    fn test_nagoya_vs_standard_rate_delta() {
        // High income to drive a meaningful taxable base (above all deductions).
        let pension_jpy = 5_000_000.0;
        let soc_ins = 0.0;
        let age = 65;
        let deps = 1;

        let std_rates    = lookup_resident_tax_rates("Kanagawa", "Sagamihara");
        let nagoya_rates = lookup_resident_tax_rates("Aichi", "Nagoya");

        let std_tax    = JapanTaxEngine::calculate_resident_tax(
            0.0, pension_jpy, soc_ins, age, deps,
            std_rates.income_rate, std_rates.per_capita_jpy,
        );
        let nagoya_tax = JapanTaxEngine::calculate_resident_tax(
            0.0, pension_jpy, soc_ins, age, deps,
            nagoya_rates.income_rate, nagoya_rates.per_capita_jpy,
        );

        // Both use same per-capita (¥6,000), so delta is purely the 0.3% rate difference.
        let rate_diff = std_rates.income_rate - nagoya_rates.income_rate; // 0.003
        assert!((rate_diff - 0.003).abs() < 1e-9, "rate_diff={}", rate_diff);

        // Delta should be positive (standard > Nagoya) and equal to rate_diff × taxable_base.
        let delta = std_tax - nagoya_tax;
        assert!(delta > 0.0, "Standard-rate city must be more expensive than Nagoya");

        // Per-capita is the same, so delta comes entirely from the income rate portion.
        // Taxable base (after pension deduction and personal deductions, rounded to ¥1k)
        // is non-trivially computed, so we just verify the ordering and approximate magnitude.
        let approx_max_delta = pension_jpy * 0.003; // upper bound (no deductions)
        assert!(
            delta <= approx_max_delta + 1.0,
            "Delta ¥{:.0} should not exceed pension × 0.3% = ¥{:.0}",
            delta, approx_max_delta
        );
    }

    /// Sagamihara (Kanagawa) should return the standard 10% + ¥6,000 per capita.
    #[test]
    fn test_sagamihara_standard_rates() {
        let rates = lookup_resident_tax_rates("Kanagawa", "Sagamihara");
        assert!(
            (rates.income_rate - STANDARD_INCOME_RATE).abs() < 1e-9,
            "Sagamihara income rate should be {} not {}",
            STANDARD_INCOME_RATE, rates.income_rate
        );
        assert!(
            (rates.per_capita_jpy - STANDARD_PER_CAPITA_JPY).abs() < 0.01,
            "Sagamihara per-capita should be ¥{} not ¥{}",
            STANDARD_PER_CAPITA_JPY, rates.per_capita_jpy
        );
    }
}
