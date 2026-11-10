//! Number formatting shared by the cards, the table and the chart.

/// Group thousands, keep at most `decimals` places, drop trailing zeros.
pub fn num(v: f64, decimals: usize) -> String {
    if !v.is_finite() {
        return "—".to_owned();
    }
    let neg = v < -0.0005;
    let a = v.abs();
    let rounded = format!("{:.*}", decimals, a);
    let (int_part, dec_part) = match rounded.split_once('.') {
        Some((i, d)) => (i.to_owned(), Some(d.to_owned())),
        None => (rounded.clone(), None),
    };
    let mut grouped = String::new();
    let digits = int_part.as_str();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    let dec = match dec_part {
        Some(d) => {
            let trimmed = d.trim_end_matches('0').to_owned();
            if trimmed.is_empty() {
                String::new()
            } else {
                format!(".{trimmed}")
            }
        }
        None => String::new(),
    };
    format!("{}{}{}", if neg { "-" } else { "" }, grouped, dec)
}

/// Whole numbers where possible, one decimal for fractional units.
pub fn qty(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        num(v, 0)
    } else {
        num(v, 1)
    }
}

/// Read a number the way it is shown: thousands separators and spaces are
/// ignored, so a cell reading "1,500" can be typed back in unchanged.
pub fn parse_qty(s: &str) -> Option<f64> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '_' && *c != '\u{00a0}')
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Week labels the way the workbook writes them: the week inside its year,
/// then the last two digits of the year — 45/26, 46/26 … 52/26, 01/27.
///
/// The sheet counts 52 weeks to a year, which is what makes 45/26 plus seven
/// weeks land on 52/26 and the next column start at 01/27. Weeks run 01–52, so
/// a 53-week year is not modelled; that matches the sheet.
pub fn period(first_week: u32, first_year: u32, i: usize) -> String {
    const WEEKS: usize = 52;
    let start = (first_week.clamp(1, WEEKS as u32) as usize - 1) + i;
    let year = first_year as usize + start / WEEKS;
    let week = start % WEEKS + 1;
    format!("{:02}/{:02}", week, year % 100)
}

/// The period label with the plan's own week number, for places with room for
/// both: "45/26 (week 1)".
pub fn period_and_week(first_week: u32, first_year: u32, i: usize) -> String {
    format!("{} (week {})", period(first_week, first_year, i), i + 1)
}

/// Compact axis labels: 12.4k, 1.2m.
pub fn compact(v: f64) -> String {
    let a = v.abs();
    if a >= 1_000_000.0 {
        format!("{}{:.1}m", if v < 0.0 { "-" } else { "" }, a / 1_000_000.0)
    } else if a >= 10_000.0 {
        format!("{}{:.0}k", if v < 0.0 { "-" } else { "" }, a / 1000.0)
    } else if a >= 1000.0 {
        format!("{}{:.1}k", if v < 0.0 { "-" } else { "" }, a / 1000.0)
    } else if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping() {
        assert_eq!(num(1234567.0, 0), "1,234,567");
        assert_eq!(num(-4200.5, 1), "-4,200.5");
        assert_eq!(num(0.0, 0), "0");
        assert_eq!(num(999.0, 0), "999");
        assert_eq!(qty(2.5), "2.5");
        assert_eq!(qty(1500.0), "1,500");
        assert_eq!(compact(12400.0), "12k");
    }

    #[test]
    fn grouped_text_parses_back() {
        assert_eq!(parse_qty("1,500"), Some(1500.0));
        assert_eq!(parse_qty(" 40 370 "), Some(40370.0));
        assert_eq!(parse_qty(""), None);
        assert_eq!(parse_qty("12x"), None);
    }

    #[test]
    fn periods_roll_over_the_new_year() {
        assert_eq!(period(45, 2026, 0), "45/26");
        assert_eq!(period(45, 2026, 7), "52/26");
        assert_eq!(period(45, 2026, 8), "01/27");
        assert_eq!(period(45, 2026, 103), "44/28");
        assert_eq!(period(1, 2030, 51), "52/30");
        assert_eq!(period(1, 2030, 52), "01/31");
        // Out-of-range settings are clamped rather than producing 00/26.
        assert_eq!(period(0, 2026, 0), "01/26");
        assert_eq!(period(99, 2026, 0), "52/26");
    }
}
