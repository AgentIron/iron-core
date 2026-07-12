//! Numeric five-field cron expression parsing and validation.
//!
//! Supports the common cron subset:
//! - Five fields: minute, hour, day-of-month, month, day-of-week
//! - Wildcard (`*`)
//! - Single numeric values
//! - Numeric ranges (`1-5`)
//! - Numeric lists (`1,3,5`)
//! - Steps (`*/15`, `1-10/2`)
//!
//! Rejected: seconds, macros (`@daily`), named months/weekdays, non-cron kinds.

use std::fmt;

// ============================================================================
// Field bounds
// ============================================================================

struct FieldBounds {
    min: u32,
    max: u32,
    label: &'static str,
}

const MINUTE: FieldBounds = FieldBounds {
    min: 0,
    max: 59,
    label: "minute",
};
const HOUR: FieldBounds = FieldBounds {
    min: 0,
    max: 23,
    label: "hour",
};
const DAY_OF_MONTH: FieldBounds = FieldBounds {
    min: 1,
    max: 31,
    label: "day-of-month",
};
const MONTH: FieldBounds = FieldBounds {
    min: 1,
    max: 12,
    label: "month",
};
const DAY_OF_WEEK: FieldBounds = FieldBounds {
    min: 0,
    max: 6,
    label: "day-of-week",
};

// ============================================================================
// Cron expression
// ============================================================================

/// A parsed and validated five-field cron expression.
///
/// Stores the set of matching values for each field. Wildcard fields match
/// every value in their valid range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CronExpression {
    minute: Vec<u32>,
    hour: Vec<u32>,
    day_of_month: Vec<u32>,
    month: Vec<u32>,
    day_of_week: Vec<u32>,
    /// Original normalized text form.
    text: String,
}

impl CronExpression {
    /// Parse and validate a five-field cron expression.
    ///
    /// Returns `Err` with a human-readable message for unsupported syntax,
    /// out-of-bounds values, zero steps, or malformed input.
    pub fn parse(input: &str) -> Result<Self, String> {
        let trimmed = input.trim();
        if trimmed.is_empty() {
            return Err("cron expression must not be empty".to_string());
        }

        // Reject macros, named entries, and extra fields.
        if trimmed.starts_with('@') {
            return Err(format!(
                "cron macros like '{}' are not supported; use five-field numeric syntax",
                trimmed
            ));
        }

        let fields: Vec<&str> = trimmed.split_whitespace().collect();
        if fields.len() != 5 {
            return Err(format!(
                "cron expression must have exactly 5 fields, got {}: '{}'",
                fields.len(),
                trimmed
            ));
        }

        let minute = parse_field(fields[0], &MINUTE)?;
        let hour = parse_field(fields[1], &HOUR)?;
        let day_of_month = parse_field(fields[2], &DAY_OF_MONTH)?;
        let month = parse_field(fields[3], &MONTH)?;
        let day_of_week = parse_field(fields[4], &DAY_OF_WEEK)?;

        // Reject named months/weekdays by checking for letters.
        for (idx, field) in fields.iter().enumerate() {
            if field.chars().any(|c| c.is_ascii_alphabetic()) {
                let labels = ["minute", "hour", "day-of-month", "month", "day-of-week"];
                return Err(format!(
                    "cron {} field '{}' contains unsupported named values; use numeric syntax",
                    labels[idx], field
                ));
            }
        }

        let text = trimmed.to_string();

        Ok(Self {
            minute,
            hour,
            day_of_month,
            month,
            day_of_week,
            text,
        })
    }

    /// The original normalized text form.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// Matching minute values (sorted).
    pub fn minutes(&self) -> &[u32] {
        &self.minute
    }

    /// Matching hour values (sorted).
    pub fn hours(&self) -> &[u32] {
        &self.hour
    }

    /// Matching day-of-month values (sorted).
    pub fn days_of_month(&self) -> &[u32] {
        &self.day_of_month
    }

    /// Matching month values (sorted).
    pub fn months(&self) -> &[u32] {
        &self.month
    }

    /// Matching day-of-week values (sorted).
    pub fn days_of_week(&self) -> &[u32] {
        &self.day_of_week
    }
}

impl fmt::Display for CronExpression {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

// ============================================================================
// Field parsing
// ============================================================================

/// Parse a single cron field into a sorted list of matching values.
fn parse_field(input: &str, bounds: &FieldBounds) -> Result<Vec<u32>, String> {
    let mut values = Vec::new();

    for item in input.split(',') {
        let parsed = parse_item(item.trim(), bounds)?;
        values.extend(parsed);
    }

    values.sort_unstable();
    values.dedup();

    Ok(values)
}

/// Parse a single comma-separated item (e.g., `*`, `5`, `1-5`, `*/15`, `1-10/2`).
fn parse_item(input: &str, bounds: &FieldBounds) -> Result<Vec<u32>, String> {
    if input.is_empty() {
        return Err(format!("cron {} field contains an empty list item", bounds.label));
    }

    let (base, step) = match input.find('/') {
        Some(pos) => {
            let step_str = &input[pos + 1..];
            let step: u32 = step_str
                .parse()
                .map_err(|_| format!("cron {} step '{}' is not a number", bounds.label, step_str))?;
            if step == 0 {
                return Err(format!("cron {} step must be greater than zero", bounds.label));
            }
            (&input[..pos], Some(step))
        }
        None => (input, None),
    };

    let range_values = parse_base(base, bounds)?;

    match step {
        Some(s) => {
            let first = range_values.first().copied().unwrap_or(0);
            Ok(range_values
                .into_iter()
                .filter(|v| (*v - first) % s == 0)
                .collect())
        }
        None => Ok(range_values),
    }
}

/// Parse the base of an item (before any `/step`): `*`, `N`, or `N-M`.
fn parse_base(base: &str, bounds: &FieldBounds) -> Result<Vec<u32>, String> {
    if base == "*" {
        return Ok((bounds.min..=bounds.max).collect());
    }

    if let Some(dash_pos) = base.find('-') {
        let lo_str = &base[..dash_pos];
        let hi_str = &base[dash_pos + 1..];
        let lo: u32 = lo_str
            .parse()
            .map_err(|_| format!("cron {} range lower bound '{}' is not a number", bounds.label, lo_str))?;
        let hi: u32 = hi_str
            .parse()
            .map_err(|_| format!("cron {} range upper bound '{}' is not a number", bounds.label, hi_str))?;

        if lo < bounds.min || lo > bounds.max {
            return Err(format!(
                "cron {} value {} is out of bounds ({}-{})",
                bounds.label, lo, bounds.min, bounds.max
            ));
        }
        if hi < bounds.min || hi > bounds.max {
            return Err(format!(
                "cron {} value {} is out of bounds ({}-{})",
                bounds.label, hi, bounds.min, bounds.max
            ));
        }
        if hi < lo {
            return Err(format!(
                "cron {} range {}-{} is descending",
                bounds.label, lo, hi
            ));
        }

        return Ok((lo..=hi).collect());
    }

    // Single value
    let val: u32 = base
        .parse()
        .map_err(|_| format!("cron {} value '{}' is not a number", bounds.label, base))?;

    if val < bounds.min || val > bounds.max {
        return Err(format!(
            "cron {} value {} is out of bounds ({}-{})",
            bounds.label, val, bounds.min, bounds.max
        ));
    }

    Ok(vec![val])
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Basic parsing ----

    #[test]
    fn wildcard_all_fields() {
        let cron = CronExpression::parse("* * * * *").unwrap();
        assert_eq!(cron.minutes().len(), 60);
        assert_eq!(cron.hours().len(), 24);
        assert_eq!(cron.days_of_month().len(), 31);
        assert_eq!(cron.months().len(), 12);
        assert_eq!(cron.days_of_week().len(), 7);
    }

    #[test]
    fn single_values() {
        let cron = CronExpression::parse("5 9 1 3 0").unwrap();
        assert_eq!(cron.minutes(), &[5]);
        assert_eq!(cron.hours(), &[9]);
        assert_eq!(cron.days_of_month(), &[1]);
        assert_eq!(cron.months(), &[3]);
        assert_eq!(cron.days_of_week(), &[0]);
    }

    #[test]
    fn ranges() {
        let cron = CronExpression::parse("1-5 0-23 1-10 1-6 0-4").unwrap();
        assert_eq!(cron.minutes(), &[1, 2, 3, 4, 5]);
        assert_eq!(cron.hours(), &(0..=23).collect::<Vec<_>>());
        assert_eq!(cron.days_of_month(), &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(cron.months(), &[1, 2, 3, 4, 5, 6]);
        assert_eq!(cron.days_of_week(), &[0, 1, 2, 3, 4]);
    }

    #[test]
    fn lists() {
        let cron = CronExpression::parse("1,3,5 0,12 1,15 1,6,12 1,3,5").unwrap();
        assert_eq!(cron.minutes(), &[1, 3, 5]);
        assert_eq!(cron.hours(), &[0, 12]);
        assert_eq!(cron.days_of_month(), &[1, 15]);
        assert_eq!(cron.months(), &[1, 6, 12]);
        assert_eq!(cron.days_of_week(), &[1, 3, 5]);
    }

    #[test]
    fn wildcard_step() {
        let cron = CronExpression::parse("*/15 * * * *").unwrap();
        assert_eq!(cron.minutes(), &[0, 15, 30, 45]);
    }

    #[test]
    fn range_step() {
        let cron = CronExpression::parse("1-10/2 * * * *").unwrap();
        assert_eq!(cron.minutes(), &[1, 3, 5, 7, 9]);
    }

    #[test]
    fn complex_expression() {
        let cron = CronExpression::parse("*/15 9-17/2 * 1,6,12 1-5").unwrap();
        assert_eq!(cron.minutes(), &[0, 15, 30, 45]);
        assert_eq!(cron.hours(), &[9, 11, 13, 15, 17]);
        assert_eq!(cron.months(), &[1, 6, 12]);
        assert_eq!(cron.days_of_week(), &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn list_deduplicates() {
        let cron = CronExpression::parse("5,5,1-3,2 * * * *").unwrap();
        assert_eq!(cron.minutes(), &[1, 2, 3, 5]);
    }

    // ---- Edge cases ----

    #[test]
    fn max_values() {
        let cron = CronExpression::parse("59 23 31 12 6").unwrap();
        assert_eq!(cron.minutes(), &[59]);
        assert_eq!(cron.hours(), &[23]);
        assert_eq!(cron.days_of_month(), &[31]);
        assert_eq!(cron.months(), &[12]);
        assert_eq!(cron.days_of_week(), &[6]);
    }

    #[test]
    fn step_one_full_range() {
        let cron = CronExpression::parse("*/1 * * * *").unwrap();
        assert_eq!(cron.minutes().len(), 60);
    }

    #[test]
    fn whitespace_trimmed() {
        let cron = CronExpression::parse("  */15   *   *   *   *  ").unwrap();
        assert_eq!(cron.as_str(), "*/15   *   *   *   *");
        assert_eq!(cron.minutes(), &[0, 15, 30, 45]);
    }

    // ---- Rejections ----

    #[test]
    fn empty_rejected() {
        assert!(CronExpression::parse("").is_err());
        assert!(CronExpression::parse("   ").is_err());
    }

    #[test]
    fn macro_rejected() {
        assert!(CronExpression::parse("@daily").is_err());
        assert!(CronExpression::parse("@hourly").is_err());
        assert!(CronExpression::parse("@reboot").is_err());
    }

    #[test]
    fn named_values_rejected() {
        assert!(CronExpression::parse("0 0 * JAN MON").is_err());
        assert!(CronExpression::parse("0 0 * * MON-FRI").is_err());
    }

    #[test]
    fn too_few_fields_rejected() {
        assert!(CronExpression::parse("* * * *").is_err());
        assert!(CronExpression::parse("* *").is_err());
    }

    #[test]
    fn too_many_fields_rejected() {
        assert!(CronExpression::parse("* * * * * *").is_err());
        assert!(CronExpression::parse("0 0 * * * 30").is_err());
    }

    #[test]
    fn out_of_bounds_rejected() {
        assert!(CronExpression::parse("60 * * * *").is_err());
        assert!(CronExpression::parse("* 24 * * *").is_err());
        assert!(CronExpression::parse("* * 0 * *").is_err());
        assert!(CronExpression::parse("* * 32 * *").is_err());
        assert!(CronExpression::parse("* * * 13 *").is_err());
        assert!(CronExpression::parse("* * * * 7").is_err());
    }

    #[test]
    fn zero_step_rejected() {
        assert!(CronExpression::parse("*/0 * * * *").is_err());
        assert!(CronExpression::parse("1-10/0 * * * *").is_err());
    }

    #[test]
    fn descending_range_rejected() {
        assert!(CronExpression::parse("5-1 * * * *").is_err());
    }

    #[test]
    fn empty_list_item_rejected() {
        assert!(CronExpression::parse("1,,3 * * * *").is_err());
        assert!(CronExpression::parse(",1 * * * *").is_err());
    }

    #[test]
    fn non_numeric_rejected() {
        assert!(CronExpression::parse("abc * * * *").is_err());
        assert!(CronExpression::parse("* * * * xyz").is_err());
    }

    #[test]
    fn display_preserves_normalized_form() {
        let cron = CronExpression::parse("*/15 9 * * 1-5").unwrap();
        assert_eq!(cron.to_string(), "*/15 9 * * 1-5");
    }
}
