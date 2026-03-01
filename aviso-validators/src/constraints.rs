//! Constraint parsing and evaluation helpers for identifier filters.
//!
//! These helpers are schema-agnostic and validate one constraint object at a time.

use anyhow::{Context, Result, bail};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub enum NumericConstraint<T> {
    Eq(T),
    In(Vec<T>),
    Gt(T),
    Gte(T),
    Lt(T),
    Lte(T),
    Between(T, T),
}

impl NumericConstraint<i64> {
    pub fn matches(&self, value: i64) -> bool {
        match self {
            NumericConstraint::Eq(v) => value == *v,
            NumericConstraint::In(values) => values.contains(&value),
            NumericConstraint::Gt(v) => value > *v,
            NumericConstraint::Gte(v) => value >= *v,
            NumericConstraint::Lt(v) => value < *v,
            NumericConstraint::Lte(v) => value <= *v,
            NumericConstraint::Between(min, max) => value >= *min && value <= *max,
        }
    }
}

impl NumericConstraint<f64> {
    pub fn matches(&self, value: f64) -> bool {
        match self {
            NumericConstraint::Eq(v) => (value - *v).abs() <= f64::EPSILON,
            NumericConstraint::In(values) => {
                values.iter().any(|v| (value - *v).abs() <= f64::EPSILON)
            }
            NumericConstraint::Gt(v) => value > *v,
            NumericConstraint::Gte(v) => value >= *v,
            NumericConstraint::Lt(v) => value < *v,
            NumericConstraint::Lte(v) => value <= *v,
            NumericConstraint::Between(min, max) => value >= *min && value <= *max,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum EnumConstraint {
    Eq(String),
    In(Vec<String>),
}

impl EnumConstraint {
    pub fn matches(&self, value: &str) -> bool {
        let canonical = value.to_lowercase();
        match self {
            EnumConstraint::Eq(v) => canonical == *v,
            EnumConstraint::In(values) => values.iter().any(|v| v == &canonical),
        }
    }
}

pub fn parse_int_constraint(
    field_name: &str,
    value: &Value,
    range: Option<&[i64; 2]>,
) -> Result<NumericConstraint<i64>> {
    let object = parse_constraint_object(field_name, value)?;
    let (operator, operand) = single_operator(field_name, object)?;

    let constraint = match operator {
        "eq" => NumericConstraint::Eq(parse_int_operand(field_name, operator, operand)?),
        "in" => NumericConstraint::In(parse_int_array_operand(field_name, operator, operand)?),
        "gt" => NumericConstraint::Gt(parse_int_operand(field_name, operator, operand)?),
        "gte" => NumericConstraint::Gte(parse_int_operand(field_name, operator, operand)?),
        "lt" => NumericConstraint::Lt(parse_int_operand(field_name, operator, operand)?),
        "lte" => NumericConstraint::Lte(parse_int_operand(field_name, operator, operand)?),
        "between" => {
            let (min, max) = parse_int_between_operand(field_name, operator, operand)?;
            NumericConstraint::Between(min, max)
        }
        _ => {
            bail!(
                "Field '{}' has invalid operator '{}'. Allowed: eq,in,gt,gte,lt,lte,between",
                field_name,
                operator
            )
        }
    };

    validate_int_constraint_against_range(field_name, &constraint, range)?;
    Ok(constraint)
}

pub fn parse_float_constraint(
    field_name: &str,
    value: &Value,
    range: Option<&[f64; 2]>,
) -> Result<NumericConstraint<f64>> {
    let object = parse_constraint_object(field_name, value)?;
    let (operator, operand) = single_operator(field_name, object)?;

    let constraint = match operator {
        "eq" => NumericConstraint::Eq(parse_float_operand(field_name, operator, operand)?),
        "in" => NumericConstraint::In(parse_float_array_operand(field_name, operator, operand)?),
        "gt" => NumericConstraint::Gt(parse_float_operand(field_name, operator, operand)?),
        "gte" => NumericConstraint::Gte(parse_float_operand(field_name, operator, operand)?),
        "lt" => NumericConstraint::Lt(parse_float_operand(field_name, operator, operand)?),
        "lte" => NumericConstraint::Lte(parse_float_operand(field_name, operator, operand)?),
        "between" => {
            let (min, max) = parse_float_between_operand(field_name, operator, operand)?;
            NumericConstraint::Between(min, max)
        }
        _ => {
            bail!(
                "Field '{}' has invalid operator '{}'. Allowed: eq,in,gt,gte,lt,lte,between",
                field_name,
                operator
            )
        }
    };

    validate_float_constraint_against_range(field_name, &constraint, range)?;
    Ok(constraint)
}

pub fn parse_enum_constraint(
    field_name: &str,
    value: &Value,
    allowed_values: &[String],
) -> Result<EnumConstraint> {
    let object = parse_constraint_object(field_name, value)?;
    let (operator, operand) = single_operator(field_name, object)?;
    let normalized_allowed: Vec<String> = allowed_values.iter().map(|v| v.to_lowercase()).collect();

    let constraint = match operator {
        "eq" => {
            let parsed = parse_enum_operand(field_name, operator, operand)?;
            validate_enum_value(field_name, &parsed, &normalized_allowed)?;
            EnumConstraint::Eq(parsed)
        }
        "in" => {
            let parsed = parse_enum_array_operand(field_name, operator, operand)?;
            for value in &parsed {
                validate_enum_value(field_name, value, &normalized_allowed)?;
            }
            EnumConstraint::In(parsed)
        }
        _ => {
            bail!(
                "Field '{}' has invalid operator '{}'. Allowed for enum constraints: eq,in",
                field_name,
                operator
            )
        }
    };

    Ok(constraint)
}

fn parse_constraint_object<'a>(
    field_name: &str,
    value: &'a Value,
) -> Result<&'a serde_json::Map<String, Value>> {
    value.as_object().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' constraint must be an object (for example {{\"gte\": 4}})",
            field_name
        )
    })
}

fn single_operator<'a>(
    field_name: &str,
    object: &'a serde_json::Map<String, Value>,
) -> Result<(&'a str, &'a Value)> {
    if object.len() != 1 {
        bail!(
            "Field '{}' constraint must contain exactly one operator",
            field_name
        );
    }
    let (operator, operand) = object
        .iter()
        .next()
        .context("constraint object unexpectedly empty")?;
    Ok((operator.as_str(), operand))
}

fn parse_int_operand(field_name: &str, operator: &str, value: &Value) -> Result<i64> {
    value.as_i64().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects integer value",
            field_name,
            operator
        )
    })
}

fn parse_int_array_operand(field_name: &str, operator: &str, value: &Value) -> Result<Vec<i64>> {
    let values = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects array of integers",
            field_name,
            operator
        )
    })?;
    if values.is_empty() {
        bail!(
            "Field '{}' operator '{}' must contain at least one value",
            field_name,
            operator
        );
    }
    values
        .iter()
        .map(|v| parse_int_operand(field_name, operator, v))
        .collect()
}

fn parse_int_between_operand(
    field_name: &str,
    operator: &str,
    value: &Value,
) -> Result<(i64, i64)> {
    let values = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects [min,max]",
            field_name,
            operator
        )
    })?;
    if values.len() != 2 {
        bail!(
            "Field '{}' operator '{}' expects exactly two values [min,max]",
            field_name,
            operator
        );
    }
    let min = parse_int_operand(field_name, operator, &values[0])?;
    let max = parse_int_operand(field_name, operator, &values[1])?;
    if min > max {
        bail!(
            "Field '{}' operator '{}' expects min <= max",
            field_name,
            operator
        );
    }
    Ok((min, max))
}

fn parse_float_operand(field_name: &str, operator: &str, value: &Value) -> Result<f64> {
    value.as_f64().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects numeric value",
            field_name,
            operator
        )
    })
}

fn parse_float_array_operand(field_name: &str, operator: &str, value: &Value) -> Result<Vec<f64>> {
    let values = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects array of numbers",
            field_name,
            operator
        )
    })?;
    if values.is_empty() {
        bail!(
            "Field '{}' operator '{}' must contain at least one value",
            field_name,
            operator
        );
    }
    values
        .iter()
        .map(|v| parse_float_operand(field_name, operator, v))
        .collect()
}

fn parse_float_between_operand(
    field_name: &str,
    operator: &str,
    value: &Value,
) -> Result<(f64, f64)> {
    let values = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects [min,max]",
            field_name,
            operator
        )
    })?;
    if values.len() != 2 {
        bail!(
            "Field '{}' operator '{}' expects exactly two values [min,max]",
            field_name,
            operator
        );
    }
    let min = parse_float_operand(field_name, operator, &values[0])?;
    let max = parse_float_operand(field_name, operator, &values[1])?;
    if min > max {
        bail!(
            "Field '{}' operator '{}' expects min <= max",
            field_name,
            operator
        );
    }
    Ok((min, max))
}

fn parse_enum_operand(field_name: &str, operator: &str, value: &Value) -> Result<String> {
    value.as_str().map(|v| v.to_lowercase()).ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects string value",
            field_name,
            operator
        )
    })
}

fn parse_enum_array_operand(
    field_name: &str,
    operator: &str,
    value: &Value,
) -> Result<Vec<String>> {
    let values = value.as_array().ok_or_else(|| {
        anyhow::anyhow!(
            "Field '{}' operator '{}' expects array of strings",
            field_name,
            operator
        )
    })?;
    if values.is_empty() {
        bail!(
            "Field '{}' operator '{}' must contain at least one value",
            field_name,
            operator
        );
    }
    values
        .iter()
        .map(|v| parse_enum_operand(field_name, operator, v))
        .collect()
}

fn validate_enum_value(field_name: &str, value: &str, allowed_values: &[String]) -> Result<()> {
    if !allowed_values.iter().any(|allowed| allowed == value) {
        bail!(
            "Field '{}' has invalid enum value '{}'. Allowed: [{}]",
            field_name,
            value,
            allowed_values.join(", ")
        );
    }
    Ok(())
}

fn validate_int_constraint_against_range(
    field_name: &str,
    constraint: &NumericConstraint<i64>,
    range: Option<&[i64; 2]>,
) -> Result<()> {
    let Some([min, max]) = range else {
        return Ok(());
    };

    let validate = |value: i64| -> Result<()> {
        if value < *min || value > *max {
            bail!(
                "Field '{}' constraint value {} is outside allowed range [{}, {}]",
                field_name,
                value,
                min,
                max
            );
        }
        Ok(())
    };

    match constraint {
        NumericConstraint::Eq(v)
        | NumericConstraint::Gt(v)
        | NumericConstraint::Gte(v)
        | NumericConstraint::Lt(v)
        | NumericConstraint::Lte(v) => validate(*v),
        NumericConstraint::In(values) => {
            for value in values {
                validate(*value)?;
            }
            Ok(())
        }
        NumericConstraint::Between(from, to) => {
            validate(*from)?;
            validate(*to)
        }
    }
}

fn validate_float_constraint_against_range(
    field_name: &str,
    constraint: &NumericConstraint<f64>,
    range: Option<&[f64; 2]>,
) -> Result<()> {
    let Some([min, max]) = range else {
        return Ok(());
    };

    let validate = |value: f64| -> Result<()> {
        if value < *min || value > *max {
            bail!(
                "Field '{}' constraint value {} is outside allowed range [{}, {}]",
                field_name,
                value,
                min,
                max
            );
        }
        Ok(())
    };

    match constraint {
        NumericConstraint::Eq(v)
        | NumericConstraint::Gt(v)
        | NumericConstraint::Gte(v)
        | NumericConstraint::Lt(v)
        | NumericConstraint::Lte(v) => validate(*v),
        NumericConstraint::In(values) => {
            for value in values {
                validate(*value)?;
            }
            Ok(())
        }
        NumericConstraint::Between(from, to) => {
            validate(*from)?;
            validate(*to)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn int_constraint_parsing_and_matching_works() {
        let constraint = parse_int_constraint("severity", &json!({"gte": 4}), Some(&[1, 7]))
            .expect("constraint should parse");
        assert!(constraint.matches(4));
        assert!(constraint.matches(7));
        assert!(!constraint.matches(3));
    }

    #[test]
    fn int_constraint_rejects_unknown_operator() {
        let result = parse_int_constraint("severity", &json!({"ge": 4}), Some(&[1, 7]));
        assert!(result.is_err());
    }

    #[test]
    fn int_constraint_rejects_out_of_range_operand() {
        let result = parse_int_constraint("severity", &json!({"gte": 9}), Some(&[1, 7]));
        assert!(result.is_err());
    }

    #[test]
    fn int_between_requires_ordered_bounds() {
        let result = parse_int_constraint("severity", &json!({"between": [6, 2]}), Some(&[1, 7]));
        assert!(result.is_err());
    }

    #[test]
    fn enum_constraint_in_works_case_insensitive() {
        let constraint = parse_enum_constraint(
            "alert_level",
            &json!({"in": ["High", "Critical"]}),
            &[
                "low".to_string(),
                "high".to_string(),
                "critical".to_string(),
            ],
        )
        .expect("enum constraint should parse");
        assert!(constraint.matches("high"));
        assert!(constraint.matches("CRITICAL"));
        assert!(!constraint.matches("low"));
    }

    #[test]
    fn enum_constraint_rejects_invalid_members() {
        let result = parse_enum_constraint(
            "alert_level",
            &json!({"in": ["high", "urgent"]}),
            &[
                "low".to_string(),
                "high".to_string(),
                "critical".to_string(),
            ],
        );
        assert!(result.is_err());
    }

    #[test]
    fn float_constraint_parses_and_matches() {
        let constraint =
            parse_float_constraint("temperature", &json!({"between": [10.5, 20.5]}), None)
                .expect("float constraint should parse");
        assert!(constraint.matches(15.0));
        assert!(!constraint.matches(22.0));
    }
}
