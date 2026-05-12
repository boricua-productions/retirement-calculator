/// Simulation-wide constants, mirroring Python's SimConstants class.
pub struct SimConstants;

impl SimConstants {
    /// Number of months for the NHI high-premium spike period after retirement.
    pub const MEDICAL_BUFFER_MONTHS: u32 = 18;

    /// Monthly NHI (National Health Insurance) premium during the spike period (Sagamihara City).
    pub const NHI_SPIKE_MONTHLY_JPY: f64 = 73_333.0;

    /// Embedded (baseline) NHI cost already included in base_expense_jpy.
    /// Only the delta above this is charged as an extra expense.
    pub const EMBEDDED_NHI_MONTHLY_JPY: f64 = 14_316.0;
}
